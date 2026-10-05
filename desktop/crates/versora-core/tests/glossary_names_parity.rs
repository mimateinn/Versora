use versora_core::glossary::{self, GlossaryError, Term};
use versora_core::names;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

fn term(term: &str, translation: &str) -> Term {
    Term {
        term: term.to_owned(),
        translation: translation.to_owned(),
    }
}

#[test]
fn safe_projects_preserve_unicode_and_reuse_existing_spelling() {
    assert_eq!(glossary::safe_project_name("  My Project  "), "My_Project");
    assert_eq!(
        glossary::safe_project_name("繁體 中文-詞庫"),
        "繁體_中文-詞庫"
    );
    assert_eq!(glossary::safe_project_name("../../a\\b:*?|<>"), "a_b");
    assert_eq!(glossary::safe_project_name("__"), "default");
    assert_eq!(glossary::safe_project_name(""), "default");
    assert_eq!(
        glossary::safe_project_name(&"甲".repeat(80)),
        "甲".repeat(64)
    );
    assert_eq!(
        glossary::canonical_project_name("my project", &strings(&["My_Project", "Other"])),
        "My_Project"
    );
    assert_eq!(
        glossary::canonical_project_name("NEW", &strings(&["My_Project"])),
        "NEW"
    );
    assert_eq!(glossary::canonical_project_name("default", &[]), "default");
}

#[test]
fn windows_device_names_are_corrected_without_losing_ordinary_names() {
    for reserved in [
        "CON", "con", "PRN", "AUX", "NUL", "COM1", "LPT9", "COM¹", "LPT³",
    ] {
        assert_eq!(
            glossary::safe_project_name(reserved),
            format!("_{reserved}")
        );
    }
    for allowed in ["Console", "COM0", "COM10", "LPT0", "LPT10", "甲"] {
        assert_eq!(glossary::safe_project_name(allowed), allowed);
    }
}

#[test]
fn glossary_legacy_import_preserves_rows_and_unicode_without_silent_corruption() {
    let json = r#"[
        {"term":" API ","translation":" 接口 "},
        ["game", "遊戲", "ignored legacy extra"],
        {"term":"loan","translation":""},
        {"term":"API","translation":"介面"},
        {"term":" ","translation":"skip"},
        {"translation":"no term"},
        ["too short"], 123,
        {"term":null,"translation":"no null text"},
        {"term":{"not":"text"},"translation":"skip"},
        {"term":"bad null target","translation":null}
    ]"#;
    let expected = vec![
        term("API", "接口"),
        term("game", "遊戲"),
        term("loan", ""),
        term("API", "介面"),
    ];
    assert_eq!(glossary::parse_glossary_json(json).unwrap(), expected);
    let serialized = glossary::glossary_json(&expected).unwrap();
    assert!(serialized.ends_with('\n'));
    assert!(serialized.contains("接口") && !serialized.contains("\\u"));
    assert_eq!(
        glossary::parse_glossary_json(&serialized).unwrap(),
        expected
    );
    assert!(matches!(
        glossary::parse_glossary_json("{}"),
        Err(GlossaryError::ExpectedArray)
    ));
    assert!(matches!(
        glossary::parse_glossary_json("[broken"),
        Err(GlossaryError::InvalidJson(_))
    ));
    assert!(glossary::parse_glossary_json("[]").unwrap().is_empty());
}

#[test]
fn glossary_scalar_legacy_values_are_compatible_and_blank_rows_are_explicit() {
    assert_eq!(
        glossary::parse_glossary_json(r#"[[12, true], [false, "no"], {"term":"keep"}]"#).unwrap(),
        vec![term("12", "True"), term("False", "no"), term("keep", "")]
    );
    let input = vec![term(" a ", "  A "), term(" ", "skip"), term("a", "second")];
    assert_eq!(
        glossary::clean_terms(&input),
        vec![term("a", "A"), term("a", "second")]
    );
    assert_eq!(input[0].term, " a ");
}

#[test]
fn glossary_is_last_and_has_explicit_precedence() {
    let rows = vec![term("API", "接口"), term("Loan", "")];
    assert_eq!(
        glossary::glossary_to_prompt_block(&rows),
        "Preferred terminology (must follow when the term appears):\n- API → 接口\n- Loan"
    );
    let prompt = glossary::append_glossary("translation rules", &rows);
    assert!(prompt.starts_with("translation rules\n\nPreferred terminology"));
    assert!(prompt.ends_with("These glossary terms override everything above."));
    assert_eq!(glossary::glossary_to_prompt_block(&[]), "");
    assert_eq!(glossary::append_glossary("unchanged", &[]), "unchanged");
}

#[test]
fn models_remain_suggestions_and_invalid_ids_fall_back_to_injected_default() {
    for id in [
        "gpt-4.1",
        "sonnet",
        "my-finetune-2026",
        "vendor:model/path_2",
        "-",
    ] {
        assert!(names::valid_model_id(id), "{id}");
    }
    for id in ["", "bad id; rm -rf", "m\n1", "甲", "quote\"", "x\\y"] {
        assert!(!names::valid_model_id(id), "{id:?}");
    }
    assert!(names::valid_model_id(&"a".repeat(64)));
    assert!(!names::valid_model_id(&"a".repeat(65)));
    assert_eq!(
        names::resolve_model("auto", Some("gpt-4.1"), Some("default")),
        None
    );
    assert_eq!(
        names::resolve_model(" Auto ", Some("gpt-4.1"), Some("default")),
        None
    );
    assert_eq!(
        names::resolve_model("openai", Some(" my-finetune-2026 "), Some("default")),
        Some("my-finetune-2026".to_owned())
    );
    assert_eq!(
        names::resolve_model("openai", Some("bad id; rm -rf"), Some("default")),
        Some("default".to_owned())
    );
    assert_eq!(names::resolve_model("codex_cli", Some(""), None), None);
    assert_eq!(
        names::resolve_model("openai", None, Some("invalid default")),
        None
    );
    assert_eq!(
        names::model_suggestions(
            Some("m1"),
            &strings(&["m2", "m1", "bad id"]),
            &strings(&["m3", "m2"])
        ),
        strings(&["m1", "m2", "m3"])
    );
}

#[test]
fn free_text_target_is_a_safe_tag_only() {
    let original = "繁體中文 / keep game terms: C:\\secret";
    assert_eq!(
        names::target_tag(original),
        "繁體中文_keep_game_terms_C_secret"
    );
    assert_eq!(original, "繁體中文 / keep game terms: C:\\secret");
    assert_eq!(names::target_tag("___"), "target");
    assert_eq!(names::target_tag(""), "target");
    assert_eq!(names::target_tag("zh-TW"), "zh-TW");
    assert_eq!(names::target_tag(&"甲".repeat(80)), "甲".repeat(64));
}

#[test]
fn output_names_cannot_carry_paths_and_keep_source_suffix() {
    assert_eq!(
        names::output_name("README.MD", "繁體中文").unwrap(),
        "README.繁體中文.md"
    );
    assert_eq!(
        names::output_name("C:\\outside\\manual.TXT", "ja").unwrap(),
        "manual.ja.txt"
    );
    assert_eq!(
        names::output_name("/outside/manual.TXT", "ja").unwrap(),
        "manual.ja.txt"
    );
    assert_eq!(
        names::output_name("archive.tar.gz", "ja").unwrap(),
        "archive.tar.ja.gz"
    );
    assert_eq!(
        names::output_name(".gitignore", "ja").unwrap(),
        ".gitignore.ja"
    );
    assert_eq!(
        names::output_name("no-extension", "").unwrap(),
        "no-extension.target"
    );
    assert_eq!(
        names::output_name("CON.txt", "ja").unwrap(),
        "file_CON.ja.txt"
    );
    for input in [
        "",
        "..",
        ".",
        "bad:name.txt",
        "bad*.txt",
        "trailing. ",
        "trail.",
        "nul\0.txt",
    ] {
        assert!(names::output_name(input, "ja").is_err(), "{input:?}");
    }
}

#[test]
fn long_output_names_respect_windows_utf16_component_limit() {
    let source = format!("{}.txt", "😀".repeat(200));
    let name = names::output_name(&source, &"繁".repeat(64)).unwrap();
    assert!(name.encode_utf16().count() <= 255);
    assert!(name.ends_with(&format!(".{}.txt", "繁".repeat(64))));
    assert!(name.starts_with('😀'));
}

#[test]
fn deterministic_unique_names_preserve_prior_outputs_and_ignore_case() {
    let base = "Manual.ja.txt";
    assert_eq!(names::unique_output_name(base, &[]).unwrap(), base);
    assert_eq!(
        names::unique_output_name(base, &strings(&["manual.JA.TXT"])).unwrap(),
        "Manual.ja (2).txt"
    );
    let existing = strings(&["Manual.ja.txt", "Manual.ja (2).txt", "Manual.ja (4).txt"]);
    assert_eq!(
        names::unique_output_name(base, &existing).unwrap(),
        "Manual.ja (3).txt"
    );
    assert_eq!(existing[0], base);
    assert!(names::unique_output_name("../unsafe", &[]).is_err());
    assert!(names::unique_output_name("CON.txt", &[]).is_err());
    let long = format!("{}.txt", "😀".repeat(125));
    let unique = names::unique_output_name(&long, std::slice::from_ref(&long)).unwrap();
    assert!(unique.encode_utf16().count() <= 255);
    assert!(unique.ends_with(" (2).txt"));
}

#[test]
fn batch_root_names_are_ascii_safe_and_bounded() {
    assert_eq!(
        names::english_job_name("folder", "My Documents"),
        "folder_My_Documents"
    );
    assert_eq!(names::english_job_name("archive", "123"), "archive_job_123");
    assert_eq!(names::english_job_name("archive", "繁體"), "archive_job");
    assert_eq!(
        names::english_job_name("../folder", "../nested/name"),
        "folder_nested_name"
    );
    assert_eq!(
        names::english_job_name("folder", &"a".repeat(80)),
        format!("folder_{}", "a".repeat(40))
    );
}
