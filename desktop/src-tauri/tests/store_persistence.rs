//! Persistence acceptance uses real owner-independent files and real Windows user-scope DPAPI.
#[path = "../src/store.rs"]
mod store;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use store::Store;
use versora_core::Term;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TestDirectory {
    path: PathBuf,
    parent: PathBuf,
}
impl TestDirectory {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let path = parent.join(format!(
            "versora-store-tests-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self { path, parent }
    }
    fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
    fn store(&self) -> Store {
        Store::new(self.child("native")).unwrap()
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.parent() == Some(self.parent.as_path())
            && self
                .path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("versora-store-tests-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn term(a: &str, b: &str) -> Term {
    Term {
        term: a.into(),
        translation: b.into(),
    }
}

#[test]
fn preferences_roundtrip_merge_aliases_and_argv_sanitization() {
    let temp = TestDirectory::new();
    let data = temp.child("native");
    let store = temp.store();
    let saved=store.save_preferences(json!({"theme":"dark","uiLang":"en","uiLangFollow":false,"concurrency":400,"perProvider":0,"project":"../中文 Project","sourceType":"zip","targetLang":"zh-Hant","targetOther":" Catalan ","modelByProvider":{"openai":"gpt-4.1-mini","codex_cli":"--dangerous","arbitrary":"run"},"chain":[{"id":"codex_cli","model":"$(touch bad)","effort":"--shell","enabled":false},{"id":"codex_cli","enabled":true},{"id":"unknown","model":"run"}],"command":"del C:"})).unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["ui_lang"], "en");
    assert_eq!(saved["ui_lang_follow"], false);
    assert_eq!(saved["concurrency"], 16);
    assert_eq!(saved["per_provider"], 1);
    assert_eq!(saved["project"], "中文_Project");
    assert_eq!(saved["source_type"], "zip");
    assert!(saved.get("command").is_none());
    assert!(saved["model_by_provider"].get("arbitrary").is_none());
    assert!(saved["model_by_provider"].get("codex_cli").is_none());
    assert_eq!(saved["chain"].as_array().unwrap().len(), 1);
    assert_eq!(saved["chain"][0]["model"], "");
    assert_eq!(saved["chain"][0]["effort"], "");
    assert_eq!(saved["chain"][0]["enabled"], false);
    store.save_preferences(json!({"purpose":"technical","theme":"nonsense","model_by_provider":{"anthropic":"claude-haiku-4-5"}})).unwrap();
    let restarted = Store::new(data).unwrap();
    assert_eq!(restarted.preferences()["theme"], "dark");
    assert_eq!(restarted.preferences()["purpose"], "technical");
    assert_eq!(
        restarted.preferences()["model_by_provider"]["openai"],
        "gpt-4.1-mini"
    );
    assert_eq!(
        restarted.preferences()["model_by_provider"]["anthropic"],
        "claude-haiku-4-5"
    );
}

#[test]
fn source_language_can_return_to_auto_and_survive_restart() {
    let temp = TestDirectory::new();
    let data = temp.child("native");
    let store = temp.store();
    store
        .save_preferences(json!({"source_choice":"en"}))
        .unwrap();
    let restarted = Store::new(data.clone()).unwrap();
    assert_eq!(restarted.preferences()["source_choice"], "en");
    assert_eq!(
        restarted
            .save_preferences(json!({"source_choice":"auto"}))
            .unwrap()["source_choice"],
        "auto"
    );
    let disk: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("projects/.sfts-ui.json")).unwrap()).unwrap();
    assert_eq!(disk["source_choice"], "auto");
    let restarted = Store::new(data.clone()).unwrap();
    assert_eq!(restarted.preferences()["source_choice"], "auto");
    restarted
        .save_preferences(json!({"sourceChoice":"ja"}))
        .unwrap();
    restarted
        .save_preferences(json!({"sourceChoice":"auto"}))
        .unwrap();
    let restarted = Store::new(data).unwrap();
    assert_eq!(restarted.preferences()["source_choice"], "auto");
    assert_eq!(restarted.preferences()["target_lang"], "en");
}

#[test]
fn reduced_motion_boolean_aliases_survive_disk_and_restart_without_coercion() {
    let temp = TestDirectory::new();
    let data = temp.child("native");
    let store = temp.store();
    assert_eq!(store.preferences()["reduced_motion"], false);

    for (patch, expected) in [
        (json!({"reduced_motion":true}), true),
        (json!({"reducedMotion":false}), false),
        (json!({"reducedMotion":true}), true),
        (json!({"reduced_motion":false}), false),
    ] {
        let restarted = Store::new(data.clone()).unwrap();
        assert_eq!(
            restarted.save_preferences(patch).unwrap()["reduced_motion"],
            expected
        );
        let disk: serde_json::Value =
            serde_json::from_slice(&fs::read(data.join("projects/.sfts-ui.json")).unwrap())
                .unwrap();
        assert_eq!(disk["reduced_motion"], expected);
        assert!(disk.get("reducedMotion").is_none());
        let restarted = Store::new(data.clone()).unwrap();
        assert_eq!(restarted.preferences()["reduced_motion"], expected);

        for alias in ["reduced_motion", "reducedMotion"] {
            for invalid in [
                json!("true"),
                json!("false"),
                json!(1),
                json!(0),
                json!(null),
                json!([]),
                json!({}),
            ] {
                let mut patch = serde_json::Map::new();
                patch.insert(alias.into(), invalid);
                assert_eq!(
                    restarted
                        .save_preferences(serde_json::Value::Object(patch))
                        .unwrap()["reduced_motion"],
                    expected
                );
                let after_restart = Store::new(data.clone()).unwrap();
                assert_eq!(after_restart.preferences()["reduced_motion"], expected);
            }
        }
    }

    // Files from older renderer versions may use the camel-case spelling directly.
    let path = data.join("projects/.sfts-ui.json");
    write(&path, br#"{"reducedMotion":true}"#);
    let restarted = Store::new(data.clone()).unwrap();
    assert_eq!(restarted.preferences()["reduced_motion"], true);
    restarted.save_preferences(json!({"theme":"dark"})).unwrap();
    let disk: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(disk["reduced_motion"], true);
    assert!(disk.get("reducedMotion").is_none());
    assert_eq!(
        Store::new(data).unwrap().preferences()["reduced_motion"],
        true
    );
}

#[test]
fn corrupt_preferences_are_visible_and_never_reset_or_overwritten() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let path = store.data_dir().join("projects/.sfts-ui.json");
    let original = b"{ owner data is broken\n";
    write(&path, original);
    assert!(Store::new(store.data_dir().to_path_buf()).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(store.save_preferences(json!({"theme":"dark"})).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".versora-write-")));
}

#[test]
fn projects_glossary_and_custom_prompt_preserve_order_and_versions() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let name = store.create_project("中文 Product").unwrap();
    assert_eq!(name, "中文_Product");
    assert_eq!(store.create_project("中文 product").unwrap(), name);
    assert_eq!(store.create_project("CON").unwrap(), "_CON");
    let rows = vec![
        term(" Button ", " 按鈕 "),
        term("Button", "重複"),
        term(" ", "ignored"),
    ];
    let saved = store.save_glossary(&name, rows).unwrap();
    assert_eq!(saved, vec![term("Button", "按鈕"), term("Button", "重複")]);
    assert_eq!(store.glossary(&name).unwrap(), saved);
    let purposes = store.purposes();
    assert_eq!(purposes.len(), 9);
    for p in &purposes[..8] {
        assert!(p["version"].as_u64().unwrap() > 0);
        assert!(!p["instructions"].as_str().unwrap().is_empty());
    }
    assert_eq!(store.save_custom(" First rule ").unwrap()["version"], 1);
    assert_eq!(store.save_custom("Second rule").unwrap()["version"], 2);
    let restarted = Store::new(store.data_dir().to_path_buf()).unwrap();
    assert_eq!(
        restarted.purposes().last().unwrap()["instructions"],
        "Second rule"
    );
    assert_eq!(restarted.glossary("中文 product").unwrap(), saved);
    let glossary = store
        .data_dir()
        .join("projects")
        .join(&name)
        .join("glossary.json");
    write(&glossary, b"{broken}");
    assert!(store.glossary(&name).is_err());
    assert!(store
        .save_glossary(&name, vec![term("replacement", "bad")])
        .is_err());
    assert_eq!(fs::read(&glossary).unwrap(), b"{broken}");
    let custom = store.data_dir().join("data/prompts/custom.md");
    write(&custom, b"version: nope\n---\nowner text");
    assert!(Store::new(store.data_dir().to_path_buf()).is_err());
    assert!(store.save_custom("replacement").is_err());
    assert_eq!(
        fs::read(&custom).unwrap(),
        b"version: nope\n---\nowner text"
    );
}

#[cfg(windows)]
#[test]
fn genuine_dpapi_roundtrip_masks_keys_and_preserves_corrupt_ciphertext() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let secret = "synthetic-store-test-secret-8b73e56b";
    let inherited = std::env::var_os("OPENAI_API_KEY");
    let status = store
        .save_provider(json!({"id":"openai","apiKey":secret,"model":"gpt-4.1-mini"}))
        .unwrap();
    assert_eq!(status["keyPresent"], true);
    assert!(!status.to_string().contains(secret));
    assert!(!store
        .provider_states()
        .iter()
        .any(|s| s.to_string().contains(secret)));
    let path = store.data_dir().join("credentials.dpapi");
    let cipher = fs::read(&path).unwrap();
    assert!(!cipher.windows(secret.len()).any(|b| b == secret.as_bytes()));
    assert!(cipher.starts_with(b"VERSORA-DPAPI\0"));
    assert!(!store.data_dir().join(".env").exists());
    assert_eq!(std::env::var_os("OPENAI_API_KEY"), inherited);
    let restarted = Store::new(store.data_dir().to_path_buf()).unwrap();
    assert_eq!(
        restarted
            .provider_configs()
            .iter()
            .find(|p| p.id == "openai")
            .unwrap()
            .api_key,
        secret
    );
    // CLI records also survive restart without accepting API secrets or arbitrary argv.
    let tool = temp.child("already-installed.exe");
    write(&tool, b"not executed by store");
    restarted.save_provider(json!({"id":"codex_cli","cliPath":tool.to_string_lossy(),"model":"gpt-5","effort":"high"})).unwrap();
    assert!(Store::new(store.data_dir().to_path_buf())
        .unwrap()
        .provider_configs()
        .iter()
        .any(|p| p.id == "codex_cli" && p.cli_path == tool.to_string_lossy()));
    assert!(restarted
        .save_provider(json!({"id":"codex_cli","cliPath":"cmd.exe /c bad"}))
        .is_err());
    assert!(restarted
        .save_provider(json!({"id":"openai","baseUrl":"http://127.0.0.1"}))
        .is_err());
    assert!(restarted
        .save_provider(
            json!({"id":"openai","baseUrl":"https://api.openai.com/v1?api_key=do-not-echo"})
        )
        .is_err());
    assert!(restarted
        .save_provider(json!({"id":"unknown","apiKey":secret}))
        .is_err());
    restarted.delete_provider("openai").unwrap();
    assert!(Store::new(store.data_dir().to_path_buf())
        .unwrap()
        .provider_configs()
        .iter()
        .find(|p| p.id == "openai")
        .unwrap()
        .api_key
        .is_empty());
    let mut damaged = fs::read(&path).unwrap();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x41;
    write(&path, &damaged);
    assert!(Store::new(store.data_dir().to_path_buf()).is_err());
    assert!(restarted
        .save_provider(json!({"id":"openai","apiKey":"new"}))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), damaged);
}

#[cfg(windows)]
#[test]
fn legacy_migration_copies_actual_bytes_without_touching_originals_or_existing_content() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let legacy = temp.child("legacy");
    let secret = "synthetic-legacy-secret-a918e2";
    let source_rows=[(".env",format!("export OPENAI_API_KEY='{secret}'\nOPENAI_MODEL=gpt-4.1-mini\nDEFAULT_PROVIDER=openai\nNEXT_PUBLIC_SECRET=do-not-import\n").into_bytes()),("projects/.sfts-ui.json",br#"{"theme":"dark","concurrency":7,"provider":"openai"}"#.to_vec()),("projects/Product/glossary.json",br#"[["Legacy","Translation"],["Legacy","Duplicate"]]"#.to_vec()),("data/prompts/custom.md",b"version: 7\n---\nOwner custom instructions\n".to_vec()),("data/outputs/subfolder/export.bin",vec![0,1,2,255]),("data/outputs/existing.txt",b"legacy output".to_vec()),("unrelated-private.txt",b"outside allowlist".to_vec())];
    for (relative, bytes) in &source_rows {
        write(&legacy.join(relative), bytes);
    }
    store
        .save_preferences(json!({"theme":"light","concurrency":2}))
        .unwrap();
    store
        .save_glossary("Product", vec![term("Current", "Keep")])
        .unwrap();
    store.save_custom("Current custom").unwrap();
    let existing_output = store.data_dir().join("data/outputs/existing.txt");
    write(&existing_output, b"current owner output");
    let prefs_before = fs::read(store.data_dir().join("projects/.sfts-ui.json")).unwrap();
    let result = store.import_legacy_data(&legacy).unwrap();
    assert_eq!(result["importedProviders"], 1);
    assert_eq!(result["plaintextEnvCopied"], false);
    assert!(!result.to_string().contains(secret));
    assert_eq!(
        fs::read(store.data_dir().join("projects/.sfts-ui.json")).unwrap(),
        prefs_before
    );
    assert_eq!(store.preferences()["theme"], "light");
    assert_eq!(
        store.glossary("Product").unwrap(),
        vec![term("Current", "Keep")]
    );
    assert_eq!(
        store.purposes().last().unwrap()["instructions"],
        "Current custom"
    );
    assert_eq!(fs::read(existing_output).unwrap(), b"current owner output");
    assert_eq!(
        fs::read(store.data_dir().join("data/outputs/subfolder/export.bin")).unwrap(),
        vec![0, 1, 2, 255]
    );
    assert!(!store.data_dir().join("unrelated-private.txt").exists());
    assert!(!store.data_dir().join(".env").exists());
    for (relative, bytes) in &source_rows {
        assert_eq!(
            &fs::read(legacy.join(relative)).unwrap(),
            bytes,
            "source file changed: {relative}"
        );
    }
    let restarted = Store::new(store.data_dir().to_path_buf()).unwrap();
    assert_eq!(
        restarted
            .provider_configs()
            .iter()
            .find(|p| p.id == "openai")
            .unwrap()
            .api_key,
        secret
    );
    restarted.delete_provider("openai").unwrap();
    assert_eq!(
        restarted.import_legacy_data(&legacy).unwrap()["importedProviders"],
        0
    );
    assert!(restarted
        .provider_configs()
        .iter()
        .find(|p| p.id == "openai")
        .unwrap()
        .api_key
        .is_empty());
}

#[cfg(windows)]
#[test]
fn legacy_first_import_retains_settings_terms_prompt_versions_and_no_plaintext_key() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let legacy = temp.child("legacy");
    write(
        &legacy.join(".env"),
        b"OPENAI_API_KEY=synthetic-first-import-secret\nDEFAULT_PROVIDER=openai\n",
    );
    write(&legacy.join("projects/.sfts-ui.json"),br#"{"theme":"dark","ui_lang":"ja","chain":[{"id":"openai","enabled":true,"model":"gpt-4.1-mini"}]}"#);
    write(
        &legacy.join("projects/default/glossary.json"),
        br#"[["term","translation"],["term","second"]]"#,
    );
    write(
        &legacy.join("data/prompts/custom.md"),
        b"version: 12\n---\nOriginal custom\n",
    );
    store.import_legacy_data(&legacy).unwrap();
    assert_eq!(store.preferences()["theme"], "dark");
    assert_eq!(store.preferences()["ui_lang"], "ja");
    assert_eq!(store.preferences()["provider"], "openai");
    assert_eq!(store.glossary("default").unwrap().len(), 2);
    assert_eq!(store.purposes().last().unwrap()["version"], 12);
    assert_eq!(store.save_custom("Next version").unwrap()["version"], 13);
    assert!(!store.data_dir().join(".env").exists());
}

#[cfg(windows)]
#[test]
fn refused_junction_cannot_import_files_from_outside_the_chosen_root() {
    use std::os::windows::process::CommandExt;
    let temp = TestDirectory::new();
    let outside = TestDirectory::new();
    let store = temp.store();
    let legacy = temp.child("legacy");
    let outputs = legacy.join("data/outputs");
    fs::create_dir_all(&outputs).unwrap();
    write(&outside.child("private.txt"), b"outside bytes");
    let junction = outputs.join("escape");
    let result = std::process::Command::new("cmd.exe")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside.path)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Could not create test-owned junction: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(store.import_legacy_data(&legacy).is_err());
    assert!(!store
        .data_dir()
        .join("data/outputs/escape/private.txt")
        .exists());
    assert_eq!(
        fs::read(outside.child("private.txt")).unwrap(),
        b"outside bytes"
    );
    fs::remove_dir(&junction).unwrap();
}

#[cfg(windows)]
#[test]
fn failed_atomic_replacement_preserves_original_bytes_and_cleans_temp_file() {
    let temp = TestDirectory::new();
    let store = temp.store();
    store.save_preferences(json!({"theme":"light"})).unwrap();
    let path = store.data_dir().join("projects/.sfts-ui.json");
    let original = fs::read(&path).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    assert!(store.save_preferences(json!({"theme":"dark"})).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".versora-write-")));
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn prompt_size_limit_cannot_commit_a_file_that_restart_refuses() {
    let temp = TestDirectory::new();
    let store = temp.store();
    let path = store.data_dir().join("data/prompts/custom.md");
    assert!(store.save_custom(&"x".repeat(256 * 1024)).is_err());
    assert!(!path.exists());
    let acceptable = "x".repeat(256 * 1024 - 128);
    store.save_custom(&acceptable).unwrap();
    assert!(fs::metadata(&path).unwrap().len() <= 256 * 1024);
    let restarted = Store::new(store.data_dir().to_path_buf()).unwrap();
    assert_eq!(
        restarted.purposes().last().unwrap()["instructions"],
        acceptable
    );
}
