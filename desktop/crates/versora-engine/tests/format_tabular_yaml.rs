use versora_engine::formats::{extract, supported_extension, write_document};

fn csv_rows(bytes: &[u8], delimiter: u8) -> Vec<Vec<String>> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(bytes)
        .records()
        .map(|row| row.unwrap().iter().map(String::from).collect())
        .collect()
}

#[test]
fn csv_preserves_bom_ragged_rows_blank_lines_and_mixed_record_endings() {
    let source = "\u{feff}id,display name,42\r\n\r\nmenu_button,\"Hello, world!\",\r\n\"line\r\nbreak\",https://example.test/\nLast sentence!\r";
    let document = extract(source.as_bytes(), ".CSV", "document").unwrap();
    assert_eq!(
        document.units,
        [
            "display name",
            "Hello, world!",
            "line\r\nbreak",
            "Last sentence!"
        ]
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let translations =
        ["顯示名稱", "你好，世界！", "第一行\r\n第二行", "最後一句！"].map(String::from);
    let output = write_document(&document, &translations).unwrap();
    assert_eq!(String::from_utf8(output.clone()).unwrap(), "\u{feff}id,顯示名稱,42\r\n\r\nmenu_button,你好，世界！,\r\n\"第一行\r\n第二行\",https://example.test/\n最後一句！\r");
    let rows = csv_rows(&output, b',');
    assert_eq!(rows[0], ["id", "顯示名稱", "42"]);
    assert_eq!(rows[1], ["menu_button", "你好，世界！", ""]);
    assert_eq!(rows[2], ["第一行\r\n第二行", "https://example.test/"]);
    assert_eq!(rows[3], ["最後一句！"]);
    assert!(write_document(&document, &[]).is_err());
}

#[test]
fn csv_and_tsv_escape_translated_delimiters_quotes_newlines_and_empty_values() {
    for (extension, delimiter) in [("csv", b','), ("tsv", b'\t')] {
        let source = format!(
            "menu_button{}Hello world!{}42\r\n",
            delimiter as char, delimiter as char
        );
        let document = extract(source.as_bytes(), extension, "game").unwrap();
        assert_eq!(document.units, ["Hello world!"]);
        for translation in ["你好\t世界, \"引號\"\r\n下一行", "", "數字 42"] {
            let output = write_document(&document, &[translation.into()]).unwrap();
            assert!(output.ends_with(b"\r\n"));
            assert_eq!(
                csv_rows(&output, delimiter),
                [vec![
                    "menu_button".to_owned(),
                    translation.to_owned(),
                    "42".to_owned()
                ]]
            );
        }
        assert!(supported_extension(extension, "document"));
    }
}

#[test]
fn csv_valid_escaped_quotes_multiline_and_terminal_empty_cells_round_trip() {
    for source in [
        "\"Hello \"\"world\"\"!\",\r\n",
        "\"\",,",
        ",\n\n",
        "\"Hello\nworld!\"\r",
        "",
        "\u{feff}",
    ] {
        let document = extract(source.as_bytes(), "csv", "document").unwrap();
        assert_eq!(
            write_document(&document, &document.units).unwrap(),
            source.as_bytes()
        );
    }
}

#[test]
fn csv_rejects_invalid_utf8_and_ambiguous_quote_framing() {
    assert!(extract(&[0xff], "csv", "document").is_err());
    for source in [
        "\"Unclosed string",
        "Hello\"world,42",
        "\"Hello world!\"suffix,42",
    ] {
        assert!(
            extract(source.as_bytes(), "csv", "document").is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn csv_limits_bound_input_cells_and_translated_cell_size() {
    let oversized_cell = "a".repeat(4 * 1024 * 1024 + 1);
    assert!(extract(oversized_cell.as_bytes(), "csv", "document")
        .unwrap_err()
        .to_string()
        .contains("4 MiB"));
    assert!(extract(",".repeat(200_000).as_bytes(), "csv", "document")
        .unwrap_err()
        .to_string()
        .contains("200000"));
    let document = extract(b"Hello world!", "csv", "document").unwrap();
    assert!(write_document(&document, &[oversized_cell])
        .unwrap_err()
        .to_string()
        .contains("4 MiB"));
}

#[test]
fn csv_rejects_aggregate_output_overflow_with_individually_valid_cells() {
    let source = (0..21)
        .map(|index| format!("Hello cell {index}!"))
        .collect::<Vec<_>>()
        .join(",");
    let document = extract(source.as_bytes(), "csv", "document").unwrap();
    assert_eq!(document.units.len(), 21);
    let translations = vec!["x".repeat(4 * 1024 * 1024); document.units.len()];
    assert!(translations
        .iter()
        .all(|cell| cell.len() <= 4 * 1024 * 1024));
    assert!(translations.iter().map(String::len).sum::<usize>() > 80 * 1024 * 1024);
    let error = write_document(&document, &translations).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Document output exceeds the 80 MB limit"),
        "unexpected rejection: {error}"
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
}

#[test]
fn csv_rejects_quote_escaping_expansion_even_below_raw_translation_budget() {
    let source = (0..11)
        .map(|index| format!("Quoted cell {index}!"))
        .collect::<Vec<_>>()
        .join(",");
    let document = extract(source.as_bytes(), "csv", "document").unwrap();
    assert_eq!(document.units.len(), 11);
    let translations = vec!["\"".repeat(4 * 1024 * 1024); document.units.len()];
    let raw_bytes: usize = translations.iter().map(String::len).sum();
    assert!(raw_bytes < 80 * 1024 * 1024);
    assert!(translations
        .iter()
        .all(|cell| cell.len() <= 4 * 1024 * 1024));
    // CSV doubles every embedded quote and adds a pair of enclosing quotes.
    let encoded_bytes = raw_bytes * 2 + document.units.len() * 2 + document.units.len() - 1;
    assert!(encoded_bytes > 80 * 1024 * 1024);
    let error = write_document(&document, &translations).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Document output exceeds the 80 MB limit"),
        "unexpected rejection: {error}"
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
}

fn yaml_documents(bytes: &[u8]) -> Vec<yaml_rust2::Yaml> {
    let raw = std::str::from_utf8(bytes)
        .unwrap()
        .strip_prefix('\u{feff}')
        .unwrap_or(std::str::from_utf8(bytes).unwrap());
    yaml_rust2::YamlLoader::load_from_str(raw).unwrap()
}

#[test]
fn yaml_translates_values_only_and_preserves_keys_order_and_scalar_types() {
    let source = "\u{feff}# Keep exact bytes without changes\r\nHello key!: Hello world!\r\ncount: 42\r\nratio: 3.14\r\nenabled: true\r\nmissing: null\r\nnested:\r\n  player_text: \"Welcome back!\"\r\n  identifier: menu_button\r\n  path: res/menu.png\r\n  values: [\"Second sentence!\", 7, false]\r\n";
    let document = extract(source.as_bytes(), "yaml", "document").unwrap();
    assert_eq!(
        document.units,
        ["Hello world!", "Welcome back!", "Second sentence!"]
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let output = write_document(
        &document,
        &[
            "你好，世界！".into(),
            "歡迎返嚟！".into(),
            "第二句！".into(),
        ],
    )
    .unwrap();
    assert!(output.starts_with(b"\xef\xbb\xbf"));
    assert!(output.ends_with(b"\r\n"));
    assert!(output
        .windows(2)
        .all(|pair| pair[1] != b'\n' || pair[0] == b'\r'));
    let parsed = yaml_documents(&output);
    let value = &parsed[0];
    assert_eq!(value["Hello key!"].as_str(), Some("你好，世界！"));
    assert_eq!(value["count"].as_i64(), Some(42));
    assert_eq!(value["ratio"].as_f64(), Some(3.14));
    assert_eq!(value["enabled"].as_bool(), Some(true));
    assert!(value["missing"].is_null());
    assert_eq!(value["nested"]["player_text"].as_str(), Some("歡迎返嚟！"));
    assert_eq!(value["nested"]["identifier"].as_str(), Some("menu_button"));
    assert_eq!(value["nested"]["path"].as_str(), Some("res/menu.png"));
    assert_eq!(value["nested"]["values"][0].as_str(), Some("第二句！"));
    assert_eq!(value["nested"]["values"][1].as_i64(), Some(7));
    assert_eq!(value["nested"]["values"][2].as_bool(), Some(false));
    let keys: Vec<_> = value
        .as_hash()
        .unwrap()
        .iter()
        .map(|(key, _)| key.as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "Hello key!",
            "count",
            "ratio",
            "enabled",
            "missing",
            "nested"
        ]
    );
    assert!(supported_extension("yml", "game"));
    assert!(supported_extension(".YAML", "document"));
    assert!(write_document(&document, &[]).is_err());
}

#[test]
fn yaml_translations_remain_strings_with_quotes_controls_and_yaml_syntax() {
    let document = extract(b"text: Hello world!", "yaml", "document").unwrap();
    for translation in [
        "true",
        "null",
        "42",
        "3.14",
        "",
        "你好: \"世界\" # 字句",
        "a\r\nb\tc\0",
    ] {
        let output = write_document(&document, &[translation.into()]).unwrap();
        assert!(!output.ends_with(b"\n"));
        assert_eq!(
            yaml_documents(&output)[0]["text"].as_str(),
            Some(translation)
        );
    }
}

#[test]
fn yaml_explicit_builtin_tags_keep_numeric_and_string_types_after_write() {
    let source = "text: !!str Hello world!\ncount: !!int 2\nfloat: !!float 2\nnegative_zero: !!float -0\nenabled: !!bool true\nnothing: !!null null\nquoted_number: !!str 42\n? !!float 3\n: Hello numeric key!\n";
    let document = extract(source.as_bytes(), "yaml", "document").unwrap();
    assert_eq!(document.units, ["Hello world!", "Hello numeric key!"]);
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let output =
        write_document(&document, &["你好，世界！".into(), "你好，數值鍵！".into()]).unwrap();
    let parsed = yaml_documents(&output);
    assert_eq!(parsed[0]["text"].as_str(), Some("你好，世界！"));
    assert_eq!(parsed[0]["count"].as_i64(), Some(2));
    assert!(matches!(&parsed[0]["float"], yaml_rust2::Yaml::Real(_)));
    assert_eq!(parsed[0]["float"].as_f64(), Some(2.0));
    assert!(parsed[0]["negative_zero"]
        .as_f64()
        .unwrap()
        .is_sign_negative());
    assert_eq!(parsed[0]["enabled"].as_bool(), Some(true));
    assert!(parsed[0]["nothing"].is_null());
    assert_eq!(parsed[0]["quoted_number"].as_str(), Some("42"));
    let (numeric_key, numeric_value) = parsed[0]
        .as_hash()
        .unwrap()
        .iter()
        .find(|(key, _)| matches!(key, yaml_rust2::Yaml::Real(_)))
        .unwrap();
    assert_eq!(numeric_key.as_f64(), Some(3.0));
    assert_eq!(numeric_value.as_str(), Some("你好，數值鍵！"));
}

#[test]
fn yaml_literal_folded_multidocument_and_complex_mapping_keys_are_supported() {
    let source = "---\r\nmessage: |-\r\n  Hello world!\r\n  Second line!\r\nfolded: >-\r\n  A folded\r\n  sentence!\r\n? [Hello key!, Second key!]\r\n: Value sentence!\r\n...\r\n---\r\n- Last sentence!\r\n- 9\r\n";
    let document = extract(source.as_bytes(), "yml", "game").unwrap();
    assert_eq!(
        document.units,
        [
            "Hello world!\nSecond line!",
            "A folded sentence!",
            "Value sentence!",
            "Last sentence!"
        ]
    );
    assert!(document.units.iter().all(|value| !value.contains("key!")));
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let translations = [
        "第一行！\n第二行！",
        "摺疊句子！",
        "數值句子！",
        "最後一句！",
    ]
    .map(String::from);
    let output = write_document(&document, &translations).unwrap();
    let before = yaml_documents(source.as_bytes());
    let after = yaml_documents(&output);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0]["message"].as_str(), Some("第一行！\n第二行！"));
    assert_eq!(after[0]["folded"].as_str(), Some("摺疊句子！"));
    let original_complex_key = before[0]
        .as_hash()
        .unwrap()
        .iter()
        .find(|(key, _)| key.is_array())
        .unwrap()
        .0;
    assert_eq!(
        after[0]
            .as_hash()
            .unwrap()
            .get(original_complex_key)
            .unwrap()
            .as_str(),
        Some("數值句子！")
    );
    assert_eq!(after[1][0].as_str(), Some("最後一句！"));
    assert_eq!(after[1][1].as_i64(), Some(9));
}

#[test]
fn yaml_bounded_aliases_expand_values_without_changing_original_or_mapping_keys() {
    let source = "# Alias presentation stays exact when unchanged\nfirst: &greeting\n  words: Hello world!\nsecond: *greeting\n";
    let document = extract(source.as_bytes(), "yaml", "document").unwrap();
    assert_eq!(document.units, ["Hello world!", "Hello world!"]);
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let output = write_document(&document, &["第一句！".into(), "第二句！".into()]).unwrap();
    let parsed = yaml_documents(&output);
    assert_eq!(parsed[0]["first"]["words"].as_str(), Some("第一句！"));
    assert_eq!(parsed[0]["second"]["words"].as_str(), Some("第二句！"));
}

#[test]
fn yaml_empty_and_scalar_only_documents_preserve_bytes() {
    for source in [
        "",
        "\u{feff}",
        "# Empty YAML\r\n",
        "---\n...\n",
        "42\r",
        "true",
        "null\r\n",
    ] {
        let document = extract(source.as_bytes(), "yaml", "document").unwrap();
        assert!(document.units.is_empty());
        assert_eq!(write_document(&document, &[]).unwrap(), source.as_bytes());
    }
    let document = extract(b"Hello world!\r", "yaml", "document").unwrap();
    let output = write_document(&document, &["獨立句子！".into()]).unwrap();
    assert!(output.ends_with(b"\r"));
    assert!(!output.contains(&b'\n'));
    assert_eq!(yaml_documents(&output)[0].as_str(), Some("獨立句子！"));
}

#[test]
fn yaml_rejects_invalid_duplicates_custom_tags_merges_and_cyclic_aliases() {
    assert!(extract(&[0xff], "yaml", "document").is_err());
    for source in [
        "text: [unterminated",
        "text: Hello world!\ntext: Another sentence!",
        "text: !Custom Hello world!",
        "!!int not_an_integer",
        "text: !!int \"2\"",
        "text: !!float >-\n  2.5\n",
        "base: &base {text: Hello world!}\nmerged: {<<: *base}",
        "self: &self [*self]",
    ] {
        assert!(
            extract(source.as_bytes(), "yaml", "document").is_err(),
            "accepted {source:?}"
        );
    }
}

#[test]
fn yaml_rejects_invalid_explicit_typed_mapping_keys_before_tree_load() {
    for (tag, value) in [
        ("int", "not_an_integer"),
        ("int", "99999999999999999999999999999999999999"),
        ("bool", "maybe"),
        ("bool", "yes"),
        ("float", "not_a_number"),
        ("null", "not_null"),
        ("null", "Null"),
    ] {
        let source = format!("? !!{tag} {value}\n: Hello world!\nHuman key!: Second sentence!\n");
        let error = extract(source.as_bytes(), "yaml", "document").unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("Invalid YAML !!{tag} scalar")),
            "unexpected rejection: {error}"
        );
        let nested = format!(
            "outer:\n  ? !!{tag} {value}\n  : Hello world!\n  Human key!: Second sentence!\n"
        );
        assert!(extract(nested.as_bytes(), "yaml", "document").is_err());
    }
}

#[test]
fn yaml_valid_explicit_typed_mapping_keys_retain_types_and_all_entries() {
    let source = "? !!int 2\n: Integer sentence!\n? !!bool true\n: Boolean sentence!\n? !!float 2\n: Float sentence!\n? !!null null\n: Null sentence!\n? !!str Human key!\n: String sentence!\n";
    let document = extract(source.as_bytes(), "yaml", "document").unwrap();
    assert_eq!(
        document.units,
        [
            "Integer sentence!",
            "Boolean sentence!",
            "Float sentence!",
            "Null sentence!",
            "String sentence!"
        ]
    );
    assert_eq!(
        write_document(&document, &document.units).unwrap(),
        source.as_bytes()
    );
    let translations = [
        "Translated integer!",
        "Translated boolean!",
        "Translated float!",
        "Translated null!",
        "Translated string!",
    ]
    .map(String::from);
    let output = write_document(&document, &translations).unwrap();
    let parsed = yaml_documents(&output);
    let mapping = parsed[0].as_hash().unwrap();
    assert_eq!(mapping.len(), 5);
    for (key, value) in [
        (yaml_rust2::Yaml::Integer(2), "Translated integer!"),
        (yaml_rust2::Yaml::Boolean(true), "Translated boolean!"),
        (yaml_rust2::Yaml::Real("2.0".into()), "Translated float!"),
        (yaml_rust2::Yaml::Null, "Translated null!"),
        (
            yaml_rust2::Yaml::String("Human key!".into()),
            "Translated string!",
        ),
    ] {
        assert_eq!(mapping.get(&key).unwrap().as_str(), Some(value));
    }
}

#[test]
fn yaml_rejects_ambiguous_plain_null_and_unsupported_core_integer_forms() {
    for scalar in [
        "Null",
        "NULL",
        "-0x1",
        "+0x2",
        "-0o7",
        "+0o7",
        "0xFFFFFFFFFFFFFFFF",
        "9223372036854775808",
        "-9223372036854775809",
        "+9223372036854775808",
    ] {
        let source = format!("message: Hello world!\nscalar: {scalar}\n");
        let error = extract(source.as_bytes(), "yaml", "document").unwrap_err();
        assert!(
            error.to_string().contains("Unsupported YAML"),
            "unexpected rejection for {scalar}: {error}"
        );
        let key = format!("? {scalar}\n: Hello world!\n");
        assert!(extract(key.as_bytes(), "yaml", "document").is_err());
    }
}

#[test]
fn yaml_quoted_and_explicit_string_scalar_controls_keep_exact_text() {
    let scalars = [
        "Null",
        "NULL",
        "-0x1",
        "+0x2",
        "-0o7",
        "+0o7",
        "0o7",
        "0x2",
        "0o7777777777777777777777",
        "0xFFFFFFFFFFFFFFFF",
        "9223372036854775808",
        "-9223372036854775809",
    ];
    for scalar in scalars {
        for spelling in [format!("'{scalar}'"), format!("!!str {scalar}")] {
            let source = format!(
                "message: Hello world!\nscalar: {spelling}\n? {spelling}\n: Keep this entry!\n"
            );
            let document = extract(source.as_bytes(), "yaml", "document").unwrap();
            assert_eq!(
                write_document(&document, &document.units).unwrap(),
                source.as_bytes()
            );
            let translations: Vec<String> = document
                .units
                .iter()
                .map(|value| {
                    if value == "Hello world!" {
                        "Changed message 世界!".into()
                    } else {
                        value.clone()
                    }
                })
                .collect();
            let output = write_document(&document, &translations).unwrap();
            let parsed = yaml_documents(&output);
            assert_eq!(parsed[0]["message"].as_str(), Some("Changed message 世界!"));
            assert_eq!(parsed[0]["scalar"].as_str(), Some(scalar));
            assert_eq!(
                parsed[0]
                    .as_hash()
                    .unwrap()
                    .get(&yaml_rust2::Yaml::String(scalar.into()))
                    .unwrap()
                    .as_str(),
                Some("Keep this entry!")
            );
            if scalar.starts_with("+0") || scalar.starts_with("0o") {
                let raw = std::str::from_utf8(&output).unwrap();
                assert!(
                    raw.contains(&format!("scalar: \"{scalar}\"")),
                    "core integer-shaped string lost quotes: {raw}"
                );
                assert!(
                    raw.contains(&format!("\"{scalar}\": Keep this entry!")),
                    "string mapping key lost quotes: {raw}"
                );
            }
        }
    }
}

#[test]
fn yaml_supported_plain_integer_boundaries_and_radix_controls_keep_integer_type() {
    let source = "message: Hello world!\nminimum: -9223372036854775808\nmaximum: 9223372036854775807\npositive: +2\nhex: 0x7FFFFFFFFFFFFFFF\noctal: 0o7\nmissing: null\n";
    let document = extract(source.as_bytes(), "yaml", "document").unwrap();
    assert_eq!(document.units, ["Hello world!"]);
    let output = write_document(&document, &["Changed message!".into()]).unwrap();
    let parsed = yaml_documents(&output);
    assert_eq!(parsed[0]["minimum"].as_i64(), Some(i64::MIN));
    assert_eq!(parsed[0]["maximum"].as_i64(), Some(i64::MAX));
    assert_eq!(parsed[0]["positive"].as_i64(), Some(2));
    assert_eq!(parsed[0]["hex"].as_i64(), Some(i64::MAX));
    assert_eq!(parsed[0]["octal"].as_i64(), Some(7));
    assert!(parsed[0]["missing"].is_null());
}

#[test]
fn yaml_limits_bound_depth_alias_expansion_documents_and_scalar_size() {
    let deep = format!("{}Hello world!{}", "[".repeat(129), "]".repeat(129));
    assert!(extract(deep.as_bytes(), "yaml", "document")
        .unwrap_err()
        .to_string()
        .contains("128"));
    let mut aliases = "a0: &a0 Hello world!\n".to_string();
    for index in 1..19 {
        aliases.push_str(&format!(
            "a{index}: &a{index} [*a{}, *a{}]\n",
            index - 1,
            index - 1
        ));
    }
    assert!(extract(aliases.as_bytes(), "yaml", "document")
        .unwrap_err()
        .to_string()
        .contains("expanded"));
    let mut linear_aliases = "a0: &a0 Hello world!\n".to_string();
    for index in 1..130 {
        linear_aliases.push_str(&format!("a{index}: &a{index} [*a{}]\n", index - 1));
    }
    assert!(extract(linear_aliases.as_bytes(), "yaml", "document")
        .unwrap_err()
        .to_string()
        .contains("128"));
    let documents = "---\nHello world!\n".repeat(65);
    assert!(extract(documents.as_bytes(), "yaml", "document")
        .unwrap_err()
        .to_string()
        .contains("64"));
    let oversized = "a".repeat(4 * 1024 * 1024 + 1);
    assert!(extract(oversized.as_bytes(), "yaml", "document")
        .unwrap_err()
        .to_string()
        .contains("4 MiB"));
    let document = extract(b"text: Hello world!", "yaml", "document").unwrap();
    assert!(write_document(&document, &[oversized])
        .unwrap_err()
        .to_string()
        .contains("4 MiB"));
}
