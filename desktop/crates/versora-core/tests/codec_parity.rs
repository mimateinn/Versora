use versora_core::codec::{self, CodecError, DedupPlan, Shape, DEFAULT_MARK, MARKS};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn reference_numbered_vectors_and_lf_continuations() {
    let items = strings(&["Hello", "two\nlines", "3. a list item"]);
    assert_eq!(
        codec::parse_numbered(&codec::numbered(&items, DEFAULT_MARK), 3, DEFAULT_MARK).unwrap(),
        items
    );
    assert_eq!(
        codec::parse_numbered("```text\n1. a\n2. b\n```", 2, DEFAULT_MARK).unwrap(),
        strings(&["a", "b"])
    );
    assert_eq!(
        codec::parse_numbered("1) a\ncontinued\n2. b", 2, DEFAULT_MARK).unwrap(),
        strings(&["a\ncontinued", "b"])
    );
    assert_eq!(
        codec::parse_numbered("1. a\u{2028}b\n2. c\u{0085}d\u{2029}e", 2, DEFAULT_MARK).unwrap(),
        strings(&["a\u{2028}b", "c\u{0085}d\u{2029}e"])
    );
}

#[test]
fn bad_replies_fail_closed_including_legacy_extra_and_duplicate_numbers() {
    for reply in [
        "Here you go:\n1. a\n2. b",
        "1. a\n3. c",
        "1. a",
        "2. b\n1. a",
        "1. a\n1. duplicated\n2. b",
        "1. a\n2. b\n3. extra",
        "1. a\n0. zero\n2. b",
        "```text\n1. a\n2. b",
        "１. a\n2. b",
        "",
        "\n \t\n",
    ] {
        assert!(
            codec::parse_numbered(reply, 2, DEFAULT_MARK).is_err(),
            "{reply:?}"
        );
    }
    assert!(matches!(
        codec::parse_numbered("1. a\n2. b\n3. extra", 2, DEFAULT_MARK),
        Err(CodecError::ItemSequence { actual: 3, .. })
    ));
    assert!(matches!(
        codec::parse_numbered(
            "99999999999999999999999999999999999999999. a",
            1,
            DEFAULT_MARK
        ),
        Err(CodecError::NumberOverflow)
    ));
    assert_eq!(
        codec::parse_numbered(" \n\t", 0, DEFAULT_MARK).unwrap(),
        Vec::<String>::new()
    );
    assert!(codec::parse_numbered("1. extra", 0, DEFAULT_MARK).is_err());
}

#[test]
fn codec_golden_layout_and_literal_marker_vectors() {
    assert_eq!(codec::encode("- a\n  - b", DEFAULT_MARK), "- a ⏎   - b");
    assert_eq!(codec::decode("a⏎b", DEFAULT_MARK), "a\nb");
    for mark in MARKS {
        for text in [
            "\tx\n\t\ty\n",
            "\n\n  a  \n\n",
            "a  \nb",
            "⏎",
            "a ⏎ b",
            "⏎\n",
            "\n⏎",
            " ⏎ \n ⏎⏎ ",
            "all ⏎ ␤ ↵ marks\n  here",
            "甲\u{2028}乙\u{0085}丙\u{2029}丁",
        ] {
            let encoded = codec::encode(text, mark);
            assert!(!encoded.contains('\n'));
            assert_eq!(
                codec::decode(&encoded, mark),
                text,
                "mark={mark} text={text:?}"
            );
        }
    }
}

#[test]
fn exhaustive_short_risky_alphabet_roundtrip() {
    fn check(prefix: &mut String, remaining: usize) {
        for mark in MARKS {
            assert_eq!(codec::decode(&codec::encode(prefix, mark), mark), *prefix);
        }
        if remaining == 0 {
            return;
        }
        for c in ['a', ' ', '\t', '\n', '⏎', '␤', '↵', '甲', '\u{2028}'] {
            prefix.push(c);
            check(prefix, remaining - 1);
            prefix.pop();
        }
    }
    check(&mut String::new(), 4);
}

#[test]
fn source_layout_stays_outside_payload_and_returns_byte_exactly() {
    let sources = strings(&[
        "- a\n  - b\n    - c",
        "Code:\n\n    def f():\n        return 1\n",
        "\tTabbed\tcell\n\t\tdeeper",
        "\n\n  lead and trail  \n\n",
        "hard break  \nnext line",
        "blank inside\n   \nafter",
        "a\r\nwindows\r\n",
        "\n\t  literal ⏎ ␤ ↵ ``` a\u{2028}b\u{0085}c  \r\n",
        "",
        " \t\r\n \n",
    ]);
    for source in sources {
        let shape = Shape::of(&source);
        assert_eq!(shape.apply_text(&shape.text).unwrap(), source);
        // A deterministic changed payload, without a provider or Python process.
        let translated = shape.text.to_uppercase();
        assert_eq!(
            shape.apply_text(&translated).unwrap(),
            source.to_uppercase()
        );
    }
    let shape = Shape::of("  first\n    second  \n");
    assert_eq!(shape.text, "first\nsecond");
    assert_eq!(
        shape.apply_text("第一\n第二").unwrap(),
        "  第一\n    第二  \n"
    );
    assert_eq!(
        Shape::of("\r\n\t  input \r\n").apply_text("輸出").unwrap(),
        "\r\n\t  輸出 \r\n"
    );
}

#[test]
fn changed_structure_and_blank_line_injection_fail_closed() {
    let shape = Shape::of("  a\n    b\n");
    assert!(matches!(
        shape.apply_text("AB"),
        Err(CodecError::LineCount {
            expected: 2,
            actual: 1
        })
    ));
    assert!(matches!(
        shape.apply_text("A\nB\nC"),
        Err(CodecError::LineCount {
            expected: 2,
            actual: 3
        })
    ));
    assert!(matches!(
        shape.apply(&strings(&["A\nextra", "B"])),
        Err(CodecError::EmbeddedLineBreak)
    ));
    assert_eq!(
        Shape::of("a\n   \nb").apply_text("A\n\nB").unwrap(),
        "A\n   \nB"
    );
    assert!(matches!(
        Shape::of("a\n   \nb").apply_text("A\ninjected\nB"),
        Err(CodecError::BlankLineChanged)
    ));
}

#[test]
fn framing_tolerance_keeps_shape_and_inner_fences() {
    let source = "  alpha\n    beta  \n";
    let shape = Shape::of(source);
    let parsed =
        codec::parse_numbered("```text\n  1. ALPHA⏎  \r\nBETA   \n```", 1, DEFAULT_MARK).unwrap();
    assert_eq!(parsed, strings(&["ALPHA\nBETA"]));
    assert_eq!(shape.apply_text(&parsed[0]).unwrap(), source.to_uppercase());
    assert_eq!(
        codec::parse_numbered("1. ``` literal\n2. code ```", 2, DEFAULT_MARK).unwrap(),
        strings(&["``` literal", "code ```"])
    );
}

#[test]
fn marker_choice_uses_first_absent_or_doubles_fallback() {
    assert_eq!(codec::choose_mark(&strings(&["plain"])), '⏎');
    assert_eq!(codec::choose_mark(&strings(&["literal ⏎"])), '␤');
    assert_eq!(
        codec::choose_mark(&strings(&["literal ⏎", "literal ␤"])),
        '↵'
    );
    let all = strings(&["all ⏎ ␤ ↵\n  kept"]);
    assert_eq!(codec::choose_mark(&all), DEFAULT_MARK);
    assert_eq!(
        codec::parse_numbered(&codec::numbered(&all, DEFAULT_MARK), 1, DEFAULT_MARK).unwrap(),
        strings(&["all ⏎ ␤ ↵\nkept"])
    );
}

#[test]
fn single_fallback_is_explicit_and_never_accepts_wrong_numbered_items() {
    assert!(codec::parse_numbered("translated", 1, DEFAULT_MARK).is_err());
    assert_eq!(
        codec::single_unnumbered("  translated  ", DEFAULT_MARK).unwrap(),
        "translated"
    );
    assert_eq!(
        codec::single_unnumbered("```text\nA ⏎ B\n```", DEFAULT_MARK).unwrap(),
        "A\nB"
    );
    for bad in ["2. wrong", "1. a\n2. b", "```\nunclosed"] {
        assert!(codec::single_unnumbered(bad, DEFAULT_MARK).is_err());
    }
}

#[test]
fn batches_count_codepoints_keep_stable_indices_and_sixty_item_cap() {
    assert!(codec::batches(&[], 100).is_empty());
    assert_eq!(
        codec::batches(&strings(&["甲乙", "丙丁"]), 20),
        vec![vec![0, 1]]
    );
    assert_eq!(
        codec::batches(&strings(&["甲乙", "丙丁"]), 19),
        vec![vec![0], vec![1]]
    );
    assert_eq!(
        codec::batches(&strings(&["oversized but kept", "b"]), 0),
        vec![vec![0], vec![1]]
    );
    let items = (0..121).map(|i| i.to_string()).collect::<Vec<_>>();
    let planned = codec::batches(&items, usize::MAX);
    assert_eq!(
        planned.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![60, 60, 1]
    );
    assert_eq!(
        planned.into_iter().flatten().collect::<Vec<_>>(),
        (0..121).collect::<Vec<_>>()
    );
}

#[test]
fn dedup_preserves_coordinate_count_case_and_blank_strings() {
    let source = strings(&["a", "", "b", "a", "A", " ", ""]);
    let plan = DedupPlan::of(&source);
    assert_eq!(plan.unique, strings(&["a", "", "b", "A", " "]));
    assert_eq!(
        plan.restore(&strings(&["甲", "", "乙", "大甲", " "]))
            .unwrap(),
        strings(&["甲", "", "乙", "甲", "大甲", " ", ""])
    );
    assert_eq!(plan.restore(&plan.unique).unwrap(), source);
    assert!(plan.restore(&strings(&["missing"])).is_err());
    assert!(plan
        .restore(&strings(&["a", "", "b", "A", " ", "extra"]))
        .is_err());
    assert_eq!(
        DedupPlan::of(&[]).restore(&[]).unwrap(),
        Vec::<String>::new()
    );
}
