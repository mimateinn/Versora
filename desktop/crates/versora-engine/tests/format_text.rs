use versora_engine::formats::{extract, supported_extension, write_document};

#[test]
fn text_and_markdown_are_utf8_preserving_and_count_checked() {
    let bytes = "\u{feff}# Hello world\r\n\r\n  Preserve indentation\r\n".as_bytes();
    for ext in ["txt", ".MD", "markdown"] {
        let doc = extract(bytes, ext, "document").unwrap();
        assert_eq!(write_document(&doc, &doc.units).unwrap(), bytes);
        assert_eq!(doc.units, ["# Hello world", "  Preserve indentation"]);
        let out = write_document(&doc, &["# 你好世界".into(), "  保留縮排".into()]).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\u{feff}# 你好世界\r\n\r\n  保留縮排\r\n"
        );
        assert!(write_document(&doc, &[]).is_err());
    }
    assert!(extract(&[0xff], "txt", "document").is_err());
    assert!(!supported_extension("exe", "document"));
    assert!(!supported_extension("js", "document"));
    assert!(supported_extension("js", "game"));
    assert!(supported_extension(".MARKDOWN", "document"));
    assert!(supported_extension("pot", "game"));
}

#[test]
fn plain_paragraph_units_preserve_bom_blank_lines_and_physical_line_endings() {
    let source = "\u{feff}\r\nFirst paragraph\r\n  Second physical line\r\n \t\r\n\r\nThird paragraph\n\nFourth paragraph\r\rLast paragraph\r";
    let document = extract(source.as_bytes(), "txt", "document").unwrap();
    assert_eq!(
        document.units,
        [
            "First paragraph\r\n  Second physical line",
            "Third paragraph",
            "Fourth paragraph",
            "Last paragraph"
        ]
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let translations = ["第一段\r\n  第二行", "第三段", "第四段", "最後一段"].map(String::from);
    assert_eq!(
        String::from_utf8(write_document(&document, &translations).unwrap()).unwrap(),
        "\u{feff}\r\n第一段\r\n  第二行\r\n \t\r\n\r\n第三段\n\n第四段\r\r最後一段\r"
    );
    let blank = "\u{feff} \t\r\n\r\n";
    let document = extract(blank.as_bytes(), "txt", "document").unwrap();
    assert!(document.units.is_empty());
    assert_eq!(write_document(&document, &[]).unwrap(), blank.as_bytes());
}

#[test]
fn plain_game_mode_filters_individual_lines_and_keeps_skipped_bytes() {
    let source = "\u{feff}menu_button\r\nHello world!\r\nres/menu.png\r\nhttps://example.test/\r\n42\r\n\r\n  Welcome back!\r\n";
    for extension in ["txt", "md", "markdown"] {
        let document = extract(source.as_bytes(), extension, "game").unwrap();
        assert_eq!(document.units, ["Hello world!", "  Welcome back!"]);
        let translated = ["你好世界！".into(), "  歡迎返嚟！".into()];
        assert_eq!(String::from_utf8(write_document(&document, &translated).unwrap()).unwrap(),
            "\u{feff}menu_button\r\n你好世界！\r\nres/menu.png\r\nhttps://example.test/\r\n42\r\n\r\n  歡迎返嚟！\r\n");
    }
}

#[test]
fn markdown_fences_indented_and_inline_code_are_never_sent_for_translation() {
    let source = "\u{feff}# Hello world\r\n\r\n```javascript\r\nconst text = \"Do not translate!\";\r\n``\r\n```\r\n\r\n~~~lua\r\nprint('Keep this code!')\r\n~~~~\r\n\r\n    print('Indented code!')\r\n\tprint('Tab code!')\r\n\r\nPlease use `menu_button` to continue.\r\nUse ``literal ` backtick`` safely!\r\n\r\n```\r\nUnclosed fence is code.\r\n";
    for extension in ["md", "markdown"] {
        for mode in ["document", "game"] {
            let document = extract(source.as_bytes(), extension, mode).unwrap();
            assert_eq!(
                write_document(&document, &document.units).unwrap(),
                source.as_bytes()
            );
            assert!(document
                .units
                .iter()
                .all(|u| !u.contains("Do not translate")
                    && !u.contains("Keep this code")
                    && !u.contains("Indented code")
                    && !u.contains("Tab code")
                    && !u.contains("Unclosed fence")
                    && !u.contains("menu_button")
                    && !u.contains("literal ` backtick")));
            let translated: Vec<_> = document
                .units
                .iter()
                .map(|s| {
                    s.replace("Hello world", "你好世界")
                        .replace("Please use", "請使用")
                        .replace("to continue.", "繼續。")
                        .replace("Use", "使用")
                        .replace("safely!", "安全操作！")
                })
                .collect();
            let output =
                String::from_utf8(write_document(&document, &translated).unwrap()).unwrap();
            assert!(output.contains("# 你好世界\r\n"));
            assert!(output
                .contains("```javascript\r\nconst text = \"Do not translate!\";\r\n``\r\n```"));
            assert!(output.contains("~~~lua\r\nprint('Keep this code!')\r\n~~~~"));
            assert!(output.contains("    print('Indented code!')\r\n\tprint('Tab code!')"));
            assert!(output.contains("請使用 `menu_button` 繼續。"));
            assert!(output.contains("使用 ``literal ` backtick`` 安全操作！"));
            assert!(output.ends_with("```\r\nUnclosed fence is code.\r\n"));
        }
    }
}

#[test]
fn paragraph_extraction_enables_real_batching_without_splitting_oversized_paragraphs() {
    let paragraphs: Vec<String> = (0..5)
        .map(|index| format!("Paragraph {index}: {}", "漢字文句 ".repeat(40)))
        .collect();
    let source = paragraphs.join("\r\n\r\n");
    let document = extract(source.as_bytes(), "txt", "document").unwrap();
    assert_eq!(document.units, paragraphs);
    let batches = versora_core::codec::batches(&document.units, 400);
    assert!(batches.len() > 1);
    assert_eq!(
        batches.iter().flatten().copied().collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    let oversized = "Oversized single paragraph: ".to_owned() + &"漢字文句 ".repeat(1000);
    let document = extract(oversized.as_bytes(), "txt", "document").unwrap();
    assert_eq!(document.units, [oversized.clone()]);
    assert_eq!(
        versora_core::codec::batches(&document.units, 400),
        vec![vec![0]]
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        oversized.as_bytes()
    );
}

#[test]
fn markdown_escaped_runs_mixed_indentation_and_multiline_code_keep_source_bytes() {
    let cases = [
        ("Lead \\``secret` after.\r\n", "secret"),
        (
            "Hello world!\r\n\r\n \tprint('secret');\r\nGood bye!\r\n",
            "secret",
        ),
        ("Read `first secret\r\nsecond secret` after.\r\n", "secret"),
        ("Lead `secret\\` after.\r\n", "secret"),
    ];
    for (source, hidden) in cases {
        for mode in ["document", "game"] {
            let document = extract(source.as_bytes(), "markdown", mode).unwrap();
            assert!(
                document.units.iter().all(|unit| !unit.contains(hidden)),
                "code reached provider input in {mode}: {:?}",
                document.units
            );
            assert_eq!(
                write_document(&document, &document.units).unwrap(),
                source.as_bytes()
            );
            let translated: Vec<_> = document
                .units
                .iter()
                .map(|unit| {
                    unit.replace("Lead", "前置")
                        .replace("Read", "閱讀")
                        .replace("after.", "之後。")
                })
                .collect();
            let output =
                String::from_utf8(write_document(&document, &translated).unwrap()).unwrap();
            assert!(output.contains(hidden));
            assert_eq!(
                output.matches("\r\n").count(),
                source.matches("\r\n").count()
            );
            if source.contains("\\``") {
                assert!(output.contains("\\``secret`"));
            }
            if source.contains(" \t") {
                assert!(output.contains(" \tprint('secret');\r\n"));
            }
            if source.contains("second secret") {
                assert!(output.contains("`first secret\r\nsecond secret`"));
            }
            if source.contains("secret\\`") {
                assert!(output.contains("`secret\\`"));
            }
        }
    }
}

#[test]
fn markdown_many_unmatched_widths_are_prose_with_exact_roundtrip() {
    let mut source = "Unmatched delimiters stay prose: ".to_string();
    for width in 1..700 {
        source.push_str(&"`".repeat(width));
        source.push_str(" ordinary text ");
    }
    let document = extract(source.as_bytes(), "md", "document").unwrap();
    assert_eq!(document.units, [source.clone()]);
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
}

#[test]
fn json_preserves_keys_numbers_order_and_formatting() {
    let source = b"{\r\n  \"first\": \"Hello world\", \"array\": [\"Hello world\", 12, true],\r\n  \"asset\": \"res/menu.png\", \"id\": \"hello_world\"\r\n}\r\n";
    let doc = extract(source, "json", "game").unwrap();
    assert_eq!(doc.units, ["Hello world", "Hello world"]);
    let translated = ["第一句 \"引號\"\n新行".into(), "第二句".into()];
    let out = write_document(&doc, &translated).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value["first"], translated[0]);
    assert_eq!(value["array"][0], translated[1]);
    assert_eq!(value["array"][1], 12);
    assert_eq!(value["asset"], "res/menu.png");
    assert!(String::from_utf8(out)
        .unwrap()
        .starts_with("{\r\n  \"first\":"));
    assert!(extract(b"{invalid", "json", "document").is_err());
}

#[test]
fn subtitles_preserve_cue_ids_timestamps_settings_and_line_endings() {
    let srt = b"1\r\n00:00:01,000 --> 00:00:02,100\r\nHello world\r\nSecond line\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nGood bye!\r\n";
    let doc = extract(srt, "srt", "document").unwrap();
    assert_eq!(doc.units, ["Hello world\r\nSecond line", "Good bye!"]);
    let out = write_document(&doc, &["你好世界\r\n第二行".into(), "再見！".into()]).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "1\r\n00:00:01,000 --> 00:00:02,100\r\n你好世界\r\n第二行\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n再見！\r\n");
    let vtt = b"WEBVTT\n\nNOTE This stays intact\n\ncue-one\n00:01.000 --> 00:02.000 align:start position:10%\nHello world\n\n";
    let doc = extract(vtt, "vtt", "document").unwrap();
    assert_eq!(doc.units, ["Hello world"]);
    let out = String::from_utf8(write_document(&doc, &["你好世界".into()]).unwrap()).unwrap();
    assert!(out.contains("NOTE This stays intact\n\ncue-one\n00:01.000 --> 00:02.000 align:start position:10%\n你好世界"));
}

#[test]
fn html_preserves_attributes_comments_scripts_and_styles_and_escapes_text() {
    let source = br#"<!doctype html><html><style>.x { content: "Do not translate"; }</style><body><!-- Keep this comment --><p title="A > B">Hello &amp; welcome!</p><script>const x = "Do not translate"; if (1 < 2) alert(x);</script></body></html>"#;
    let doc = extract(source, "html", "document").unwrap();
    assert_eq!(doc.units, ["Hello & welcome!"]);
    let out =
        String::from_utf8(write_document(&doc, &["你好 & <大家>！".into()]).unwrap()).unwrap();
    assert!(out.contains("<p title=\"A > B\">你好 &amp; &lt;大家&gt;！</p>"));
    assert!(out.contains("const x = \"Do not translate\"; if (1 < 2) alert(x);"));
    assert!(out.contains("<!-- Keep this comment -->"));
}

#[test]
fn scripts_skip_comments_and_identifiers_and_escape_translated_literals() {
    for extension in ["lua", "js", "ts", "gd"] {
        let prefix = match extension {
            "lua" => "--",
            "gd" => "#",
            _ => "//",
        };
        let source = format!("{prefix} \"Do not translate comment\"\nname = 'Hello world'\nasset = \"res/menu.png\"\nkey = \"hello_world\"\nescaped = \"He said \\\"hello\\\"!\"\n");
        let doc = extract(source.as_bytes(), extension, "game").unwrap();
        assert_eq!(doc.units, ["Hello world", "He said \"hello\"!"]);
        let translated = ["佢話 '你好'！\\路徑\n新行".into(), "包含 \"引號\"！".into()];
        let out = write_document(&doc, &translated).unwrap();
        let reparsed = extract(&out, extension, "game").unwrap();
        assert_eq!(reparsed.units, translated);
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("asset = \"res/menu.png\""));
        assert!(out.contains("key = \"hello_world\""));
        assert!(out.contains(&format!("{prefix} \"Do not translate comment\"")));
    }
}

#[test]
fn po_handles_unicode_multiline_headers_and_plurals_without_changing_ids() {
    let source = include_bytes!("fixtures/messages.po");
    let doc = extract(source, "po", "document").unwrap();
    assert_eq!(extract(source, "pot", "document").unwrap().units, doc.units);
    assert_eq!(
        doc.units,
        [
            "Hello world\nSecond line",
            "One apple",
            "Many apples",
            "現有譯文！"
        ]
    );
    let out = write_document(
        &doc,
        &[
            "你好世界\n第二行".into(),
            "一個蘋果".into(),
            "多個蘋果".into(),
            "更新譯文！".into(),
        ],
    )
    .unwrap();
    let text = String::from_utf8(out.clone()).unwrap();
    assert!(text.contains("msgid \"\"\n\"Hello world\\n\"\n\"Second line\""));
    assert!(text.contains("msgstr \"你好世界\\n第二行\""));
    assert!(text.contains("msgstr[0] \"一個蘋果\"\nmsgstr[1] \"多個蘋果\""));
    assert!(text.contains("Content-Type: text/plain; charset=UTF-8\\n"));
    assert_eq!(
        extract(&out, "po", "document").unwrap().units[3],
        "更新譯文！"
    );
}

#[test]
fn xliff_preserves_inline_codes_namespaces_ids_and_creates_targets() {
    let source = include_bytes!("fixtures/localization.xlf");
    let doc = extract(source, "xlf", "document").unwrap();
    assert_eq!(doc.units.len(), 2);
    assert!(doc.units[0].contains("⟦XLIFF:0⟧"));
    let translated: Vec<String> = doc
        .units
        .iter()
        .map(|s| {
            s.replace("Hello ", "你好 ")
                .replace("world", "世界")
                .replace("Click here!", "按這裏！")
        })
        .collect();
    let out = String::from_utf8(write_document(&doc, &translated).unwrap()).unwrap();
    assert!(out.contains("<x:source>Hello <x:g id=\"b\">world</x:g><x:x id=\"line\"/></x:source>"));
    assert!(out.contains("<x:target state=\"needs-translation\">你好 <x:g id=\"b\">世界</x:g><x:x id=\"line\"/></x:target>"));
    assert!(out.contains("<x:target>按這裏！</x:target>"));
    assert!(out.contains("id=\"second\""));
    assert!(write_document(&doc, &["Dropped inline tags".into(), "按這裏！".into()]).is_err());
}

#[test]
fn html_decodes_full_named_entities_and_xliff_cdata_without_loosening_xml() {
    let html = b"<p>Hello &trade; &NotEqualTilde; &#x4e16;&#30028;!</p>";
    let doc = extract(html, "html", "document").unwrap();
    assert_eq!(doc.units, ["Hello ™ ≂\u{338} 世界!"]);
    let xliff = br#"<xliff><file><body><trans-unit id="cdata"><source><![CDATA[Hello <world> & welcome!]]></source><target/></trans-unit></body></file></xliff>"#;
    let doc = extract(xliff, "xlf", "document").unwrap();
    assert_eq!(doc.units, ["Hello <world> & welcome!"]);
    let out =
        String::from_utf8(write_document(&doc, &["你好 <世界> & 歡迎！".into()]).unwrap()).unwrap();
    assert!(out.contains("<target>你好 &lt;世界&gt; &amp; 歡迎！</target>"));
    let invalid = b"<xliff><file><source>Hello &trade; world!</source></file></xliff>";
    assert!(extract(invalid, "xlf", "document").is_err());
}
