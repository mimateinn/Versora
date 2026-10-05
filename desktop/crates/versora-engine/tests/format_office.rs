use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};
use versora_engine::formats::{extract, write_document};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.set_comment("Versora native structural format fixture");
    for (name, contents) in parts {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let mut content = Vec::new();
        file.read_to_end(&mut content).unwrap();
        out.insert(file.name().into(), content);
    }
    out
}
fn export_test_artifact(name: &str, bytes: &[u8]) {
    if let Some(directory) = std::env::var_os("VERSORA_FORMAT_TEST_EXPORT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}
const ROOT_RELS: &[u8] = br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOCX_TYPES: &[u8] = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

#[test]
fn docx_translates_paragraphs_and_table_cells_preserving_package_styles_and_order() {
    let source = package(&[
        ("[Content_Types].xml", DOCX_TYPES), ("_rels/.rels", ROOT_RELS),
        ("word/document.xml", include_bytes!("fixtures/document.xml")),
        ("word/styles.xml", br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading 1"/></w:style></w:styles>"#),
        ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#),
        ("docProps/custom.xml", br#"<Properties><Keep>This metadata survives.</Keep></Properties>"#),
    ]);
    let doc = extract(&source, "docx", "document").unwrap();
    assert_eq!(
        doc.units,
        [
            "Hello world",
            "Left cell text",
            "Right cell text",
            "Trailing paragraph"
        ]
    );
    assert_eq!(write_document(&doc, &doc.units).unwrap(), source);
    let translated: Vec<String> = ["你好世界 & <引號>", "左邊儲存格", "右邊儲存格", "尾段文字"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let result = write_document(&doc, &translated).unwrap();
    export_test_artifact("translated.docx", &result);
    let before = members(&source);
    let after = members(&result);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &before {
        if name != "word/document.xml" {
            assert_eq!(&after[name], bytes, "unmodified member {name}");
        }
    }
    let xml = String::from_utf8(after["word/document.xml"].clone()).unwrap();
    assert!(xml.contains("<w:pStyle w:val=\"Heading1\"/>"));
    assert!(xml.contains("<w:rPr><w:b/></w:rPr>"));
    assert!(
        xml.contains("<w:tblGrid><w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"3000\"/></w:tblGrid>")
    );
    assert!(xml.contains("<w:tbl>"));
    assert!(xml.contains("你好世界 &amp; &lt;引號&gt;"));
    assert_eq!(
        extract(&result, "docx", "document").unwrap().units,
        translated
    );
}

#[test]
fn xlsx_translates_shared_and_inline_rich_strings_and_preserves_formulas_and_numerics() {
    let types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#;
    let source = package(&[
        ("[Content_Types].xml", types),
        ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
        ("xl/workbook.xml", br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Original Sheet" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
        ("xl/_rels/workbook.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#),
        ("xl/worksheets/sheet1.xml", include_bytes!("fixtures/worksheet.xml")),
        ("xl/sharedStrings.xml", br#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><r><rPr><b/></rPr><t>Hello </t></r><r><t>world</t></r></si><si><t>Unused metadata text</t></si></sst>"#),
    ]);
    let doc = extract(&source, "xlsx", "document").unwrap();
    assert_eq!(doc.units, ["Inline cell text", "Hello world"]);
    let translated = ["內嵌儲存格 & <文字>".into(), "你好世界".into()];
    let result = write_document(&doc, &translated).unwrap();
    export_test_artifact("translated.xlsx", &result);
    let before = members(&source);
    let after = members(&result);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    assert_eq!(after["xl/workbook.xml"], before["xl/workbook.xml"]);
    let xml = String::from_utf8(after["xl/worksheets/sheet1.xml"].clone()).unwrap();
    assert!(xml.contains("<c r=\"B1\"><v>42.5</v></c>"));
    assert!(xml.contains("<c r=\"B2\" t=\"str\"><f>CONCATENATE(A1,\" Literal formula text\")</f><v>Do not translate formula cache</v></c>"));
    assert!(xml.contains("<pane xSplit=\"1\" ySplit=\"1\" topLeftCell=\"B2\" state=\"frozen\"/>"));
    let shared = String::from_utf8(after["xl/sharedStrings.xml"].clone()).unwrap();
    assert!(shared.contains("<rPr><b/></rPr>"));
    assert!(shared.contains("Unused metadata text"));
    assert_eq!(
        extract(&result, "xlsx", "document").unwrap().units,
        translated
    );
}

#[test]
fn malformed_office_packages_and_xml_fail_without_fallback() {
    assert!(extract(b"Not a zip", "docx", "document").is_err());
    let broken = package(&[
        ("[Content_Types].xml", DOCX_TYPES),
        ("word/document.xml", b"<w:document><w:p>unclosed"),
    ]);
    assert!(extract(&broken, "docx", "document").is_err());
    let dtd = package(&[("[Content_Types].xml", DOCX_TYPES), ("word/document.xml", b"<!DOCTYPE w:document [<!ENTITY x SYSTEM 'file:///secret'>]><w:document>&x;</w:document>")]);
    assert!(extract(&dtd, "docx", "document").is_err());
}

#[test]
fn docx_retains_original_tabs_breaks_and_translates_single_word_headings() {
    let original = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Title</w:t><w:tab/><w:t>Before break</w:t><w:br/><w:t>After break</w:t></w:r></w:p></w:body></w:document>"#;
    let source = package(&[
        ("[Content_Types].xml", DOCX_TYPES),
        ("word/document.xml", original),
    ]);
    let doc = extract(&source, "docx", "document").unwrap();
    assert_eq!(doc.units, ["Title", "Before break", "After break"]);
    let translated = ["標題".into(), "  換行前  ".into(), "換行後".into()];
    let result = write_document(&doc, &translated).unwrap();
    let xml = String::from_utf8(members(&result)["word/document.xml"].clone()).unwrap();
    assert_eq!(xml.matches("<w:tab/>").count(), 1);
    assert_eq!(xml.matches("<w:br/>").count(), 1);
    assert!(xml.contains("<w:t xml:space=\"preserve\">  換行前  </w:t>"));
    assert_eq!(
        extract(&result, "docx", "document").unwrap().units,
        translated
    );
    assert!(write_document(&doc, &["標題\0".into(), "換行前".into(), "換行後".into()]).is_err());
}
