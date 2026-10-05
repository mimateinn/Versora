//! Native user-data persistence. Secrets never cross IPC and never enter process environment.
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use versora_core::{
    glossary::{
        canonical_project_name, clean_terms, glossary_json, parse_glossary_json, safe_project_name,
    },
    Term,
};
use versora_engine::providers::{
    default_base_url, default_model, validate_endpoint, ProviderConfig,
};

const IDS: [&str; 7] = [
    "openai",
    "anthropic",
    "gemini",
    "xai",
    "claude_cli",
    "grok_cli",
    "codex_cli",
];
const LANGS: [&str; 12] = [
    "zh-Hant", "zh-Hans", "en", "ja", "ko", "es", "fr", "de", "pt", "vi", "th", "id",
];
const PRESETS: [(&str, &str); 8] = [
    ("general", include_str!("../../../prompts/general.md")),
    ("technical", include_str!("../../../prompts/technical.md")),
    ("ui", include_str!("../../../prompts/ui.md")),
    ("subtitles", include_str!("../../../prompts/subtitles.md")),
    ("game", include_str!("../../../prompts/game.md")),
    ("legal", include_str!("../../../prompts/legal.md")),
    ("academic", include_str!("../../../prompts/academic.md")),
    ("business", include_str!("../../../prompts/business.md")),
];
const JSON_CAP: u64 = 4 * 1024 * 1024;
const PROMPT_CAP: u64 = 256 * 1024;
const CREDENTIAL_MAGIC: &[u8] = b"VERSORA-DPAPI\0";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct State {
    preferences: Value,
    providers: BTreeMap<String, ProviderConfig>,
    custom: (u64, String),
}
pub struct Store {
    data_dir: PathBuf,
    state: Mutex<State>,
}

impl Store {
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        if !data_dir.is_absolute() {
            return Err("User-data directory must be absolute".into());
        }
        reject_links(&data_dir)?;
        // Validate owner files before creating anything or applying defaults.
        let preferences = merge_preferences(
            default_preferences(),
            read_preferences(&data_dir.join("projects/.sfts-ui.json"))?,
        )?;
        let providers = read_credentials(&data_dir.join("credentials.dpapi"))?;
        let custom = read_custom(&data_dir.join("data/prompts/custom.md"))?;
        for part in ["", "projects", "data/prompts", "data/outputs"] {
            ensure_dir(&data_dir.join(part))?;
        }
        let data_dir = data_dir
            .canonicalize()
            .map_err(|_| "Cannot resolve user-data directory")?;
        Ok(Self {
            data_dir,
            state: Mutex::new(State {
                preferences,
                providers,
                custom,
            }),
        })
    }
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    pub fn preferences(&self) -> Value {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .preferences
            .clone()
    }
    pub fn save_preferences(&self, patch: Value) -> Result<Value, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let path = self.data_dir.join("projects/.sfts-ui.json");
        let current = merge_preferences(default_preferences(), read_preferences(&path)?)?;
        let next = merge_preferences(current, patch)?;
        atomic_write(&path, &json_bytes(&next)?, true)?;
        state.preferences = next.clone();
        Ok(next)
    }
    pub fn projects(&self) -> Result<Vec<String>, String> {
        project_names(&self.data_dir.join("projects"))
    }
    pub fn glossary(&self, project: &str) -> Result<Vec<Term>, String> {
        let name = canonical_project_name(project, &self.projects()?);
        let path = self
            .data_dir
            .join("projects")
            .join(name)
            .join("glossary.json");
        read_glossary(&path)
    }
    pub fn create_project(&self, name: &str) -> Result<String, String> {
        let _state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        self.ensure_project(name)
    }
    fn ensure_project(&self, name: &str) -> Result<String, String> {
        let name = canonical_project_name(name, &self.projects()?);
        let directory = self.data_dir.join("projects").join(&name);
        ensure_dir(&directory)?;
        let glossary = directory.join("glossary.json");
        if glossary
            .try_exists()
            .map_err(|_| "Cannot inspect project glossary")?
        {
            read_glossary(&glossary)?;
        } else {
            atomic_write(&glossary, b"[]\n", false)?;
        }
        Ok(name)
    }
    pub fn save_glossary(&self, project: &str, entries: Vec<Term>) -> Result<Vec<Term>, String> {
        let _state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let name = self.ensure_project(project)?;
        let entries = clean_terms(&entries);
        let text = glossary_json(&entries).map_err(|_| "Cannot encode terminology")?;
        if text.len() as u64 > JSON_CAP {
            return Err("Glossary exceeds the 4 MiB limit".into());
        }
        atomic_write(
            &self
                .data_dir
                .join("projects")
                .join(name)
                .join("glossary.json"),
            text.as_bytes(),
            true,
        )?;
        Ok(entries)
    }
    pub fn purposes(&self) -> Vec<Value> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut rows: Vec<Value> = PRESETS
            .iter()
            .map(|(id, text)| {
                let (version, body) = parse_prompt(text).expect("validated shipped prompt");
                json!({"id":id,"version":version,"instructions":body})
            })
            .collect();
        let (version, instructions) = if state.custom.1.is_empty() {
            parse_prompt(PRESETS[0].1).expect("validated shipped prompt")
        } else {
            state.custom.clone()
        };
        rows.push(json!({"id":"custom","version":version,"instructions":instructions}));
        rows
    }
    pub fn save_custom(&self, instructions: &str) -> Result<Value, String> {
        if instructions.len() as u64 > PROMPT_CAP {
            return Err("Custom prompt exceeds the 256 KiB limit".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let path = self.data_dir.join("data/prompts/custom.md");
        let old = read_custom(&path)?;
        let version = old
            .0
            .checked_add(1)
            .ok_or("Custom prompt version is exhausted")?;
        let body = instructions.trim().to_owned();
        let text = format!("version: {version}\n---\n{body}\n");
        if text.len() as u64 > PROMPT_CAP {
            return Err("Custom prompt including its header exceeds the 256 KiB limit".into());
        }
        atomic_write(&path, text.as_bytes(), true)?;
        state.custom = (version, body.clone());
        Ok(json!({"id":"custom","version":version,"instructions":body}))
    }
    pub fn provider_configs(&self) -> Vec<ProviderConfig> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut configs: Vec<_> = IDS
            .iter()
            .map(|id| {
                state
                    .providers
                    .get(*id)
                    .cloned()
                    .unwrap_or_else(|| default_config(id))
            })
            .collect();
        if demo_enabled() {
            configs.push(default_config("demo"));
        }
        configs
    }
    pub fn provider_states(&self) -> Vec<Value> {
        let prefs = self.preferences();
        self.provider_configs()
            .iter()
            .map(|config| {
                let mut row = masked_provider_state(config);
                if let Some(chain) = prefs["chain"].as_array() {
                    if let Some(item) = chain.iter().find(|v| v["id"] == config.id) {
                        row["enabled"] = item["enabled"].clone();
                    }
                }
                row
            })
            .collect()
    }
    pub fn save_provider(&self, value: Value) -> Result<Value, String> {
        let input = value
            .as_object()
            .ok_or("Provider settings must be an object")?;
        let id = input
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Provider id is required")?;
        if !IDS.contains(&id) {
            return Err("Unknown saved provider".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let path = self.data_dir.join("credentials.dpapi");
        let mut rows = read_credentials(&path)?;
        let mut config = rows.get(id).cloned().unwrap_or_else(|| default_config(id));
        patch_provider(&mut config, input)?;
        rows.insert(id.into(), config.clone());
        write_credentials(&path, &rows)?;
        state.providers = rows;
        Ok(masked_provider_state(&config))
    }
    pub fn delete_provider(&self, id: &str) -> Result<Value, String> {
        if !IDS.contains(&id) {
            return Err("Unknown saved provider".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let path = self.data_dir.join("credentials.dpapi");
        let mut rows = read_credentials(&path)?;
        // An empty saved record also preserves the owner's deletion when an old .env is imported.
        let config = default_config(id);
        rows.insert(id.into(), config.clone());
        write_credentials(&path, &rows)?;
        state.providers = rows;
        Ok(masked_provider_state(&config))
    }
    pub fn import_legacy_data(&self, root: &Path) -> Result<Value, String> {
        reject_links(root)?;
        if !root.is_dir() {
            return Err("Legacy source must be a directory".into());
        }
        let root = root
            .canonicalize()
            .map_err(|_| "Cannot resolve legacy source")?;
        if root.starts_with(&self.data_dir) || self.data_dir.starts_with(&root) {
            return Err("Legacy source and destination must be separate directories".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "User-data store is busy after an earlier error")?;
        let prefs_path = self.data_dir.join("projects/.sfts-ui.json");
        let old_preferences = read_preferences(&prefs_path)?;
        let mut providers = read_credentials(&self.data_dir.join("credentials.dpapi"))?;
        let mut source_preferences = read_preferences(&root.join("projects/.sfts-ui.json"))?;
        let env = read_legacy_env(&root.join(".env"))?;
        if source_preferences.get("provider").is_none() {
            if let Some(id) = env.get("DEFAULT_PROVIDER").filter(|id| known_provider(id)) {
                source_preferences["provider"] = json!(id);
            }
        }
        let mut incoming = Vec::new();
        let mut key_names = Vec::new();
        for id in IDS {
            if providers.contains_key(id) {
                continue;
            }
            if let Some(config) = legacy_config(id, &env)? {
                if !config.api_key.is_empty() {
                    key_names.push(format!("{}_API_KEY", id.to_ascii_uppercase()));
                }
                incoming.push(config);
            }
        }
        let mut glossaries = Vec::new();
        for name in project_names(&root.join("projects"))? {
            let source = root.join("projects").join(&name).join("glossary.json");
            let rows = read_glossary(&source)?;
            glossaries.push((safe_project_name(&name), rows));
        }
        let custom_source = root.join("data/prompts/custom.md");
        let custom = if custom_source
            .try_exists()
            .map_err(|_| "Cannot inspect legacy custom prompt")?
        {
            Some(read_custom(&custom_source)?)
        } else {
            None
        };
        let mut output_files = Vec::new();
        let mut total = 0;
        collect_outputs(
            &root.join("data/outputs"),
            Path::new(""),
            &mut output_files,
            &mut total,
        )?;
        read_custom(&self.data_dir.join("data/prompts/custom.md"))?;
        for (name, _) in &glossaries {
            let name = canonical_project_name(name, &self.projects()?);
            reject_links(
                &self
                    .data_dir
                    .join("projects")
                    .join(name)
                    .join("glossary.json"),
            )?;
        }
        for (_, relative) in &output_files {
            reject_links(&self.data_dir.join("data/outputs").join(relative))?;
        }
        // Validate records and path boundaries before copying. Every write is atomic and an
        // interrupted import can be retried without replacing existing destination files.
        let has_source_preferences = !source_preferences.as_object().is_some_and(Map::is_empty);
        let merged_source = merge_preferences(source_preferences, old_preferences)?;
        let next_preferences = merge_preferences(default_preferences(), merged_source)?;
        let mut copied = 0usize;
        let mut skipped = 0usize;
        if has_source_preferences {
            if prefs_path
                .try_exists()
                .map_err(|_| "Cannot inspect destination preferences")?
            {
                skipped += 1;
            } else {
                atomic_write(&prefs_path, &json_bytes(&next_preferences)?, false)?;
                copied += 1;
                state.preferences = next_preferences;
            }
        }
        for (name, rows) in glossaries {
            let name = canonical_project_name(&name, &self.projects()?);
            let destination = self
                .data_dir
                .join("projects")
                .join(name)
                .join("glossary.json");
            reject_links(&destination)?;
            if destination
                .try_exists()
                .map_err(|_| "Cannot inspect destination glossary")?
            {
                skipped += 1;
                continue;
            }
            atomic_write(
                &destination,
                glossary_json(&rows)
                    .map_err(|_| "Cannot encode legacy terminology")?
                    .as_bytes(),
                false,
            )?;
            copied += 1;
        }
        if let Some((version, body)) = custom {
            let destination = self.data_dir.join("data/prompts/custom.md");
            reject_links(&destination)?;
            if destination
                .try_exists()
                .map_err(|_| "Cannot inspect destination custom prompt")?
            {
                skipped += 1;
            } else {
                atomic_write(
                    &destination,
                    format!("version: {version}\n---\n{body}\n").as_bytes(),
                    false,
                )?;
                state.custom = (version, body);
                copied += 1;
            }
        }
        for (source, relative) in output_files {
            let destination = self.data_dir.join("data/outputs").join(relative);
            reject_links(&destination)?;
            if destination
                .try_exists()
                .map_err(|_| "Cannot inspect destination output")?
            {
                skipped += 1;
                continue;
            }
            atomic_write(
                &destination,
                &read_bytes(&source, 128 * 1024 * 1024)?,
                false,
            )?;
            copied += 1;
        }
        let imported_providers = incoming.len();
        if imported_providers > 0 {
            for config in incoming {
                providers.insert(config.id.clone(), config);
            }
            write_credentials(&self.data_dir.join("credentials.dpapi"), &providers)?;
            state.providers = providers;
        }
        Ok(
            json!({"sourcePath":root.display().to_string(),"copiedFiles":copied,"skippedFiles":skipped,"importedProviders":imported_providers,"importedKeyNames":key_names,"plaintextEnvCopied":false}),
        )
    }
}

pub fn masked_provider_state(config: &ProviderConfig) -> Value {
    let cli = config.id.ends_with("_cli");
    let key = !config.api_key.is_empty();
    let configured = if config.id == "demo" {
        demo_enabled()
    } else if cli {
        !config.cli_path.is_empty()
    } else {
        key
    };
    let name = match config.id.as_str() {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "gemini" => "Gemini",
        "xai" => "xAI",
        "claude_cli" => "Claude Code",
        "grok_cli" => "Grok CLI",
        "codex_cli" => "Codex",
        "demo" => "Offline demo",
        _ => "Unknown",
    };
    json!({"id":config.id,"name":name,"kind":if cli {"cli"} else {"api"},"configured":configured,"status":if configured {"configured"} else {"unconfigured"},"model":config.model,"effort":config.effort,"enabled":true,"keyPresent":key,"cliPath":config.cli_path,"baseUrl":config.base_url,"probePerformed":false,"nativeDetected":null,"signedIn":null,"transportVerified":false,"availableForAttempt":configured && (!cli || config.id=="demo"),"detail":if config.id=="demo" {"Explicit offline demonstration; does not translate"} else {"Native availability and live translation require a separate probe/test"}})
}
fn demo_enabled() -> bool {
    ["VERSORA_DEMO", "SFTS_DEMO"]
        .iter()
        .any(|key| std::env::var(key).as_deref() == Ok("1"))
}
fn known_provider(id: &str) -> bool {
    IDS.contains(&id) || matches!(id, "auto" | "demo")
}
fn default_config(id: &str) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        model: default_model(id).into(),
        base_url: default_base_url(id).into(),
        ..Default::default()
    }
}
fn default_preferences() -> Value {
    json!({"theme":"light","hologram_tone":"light","ui_lang":"zh-Hant","ui_lang_follow":true,"reduced_motion":false,"provider":"auto","model_by_provider":{},"chain":IDS.iter().map(|id|json!({"id":id,"model":"","effort":"","enabled":true})).collect::<Vec<_>>(),"concurrency":3,"per_provider":1,"purpose":"general","project":"default","source_type":"file","content_mode":"document","target_lang":"en","source_choice":"auto","target_other":""})
}
fn safe_model(value: &str) -> bool {
    value.is_empty() || (!value.starts_with('-') && versora_core::names::valid_model_id(value))
}
fn language_code(value: &str) -> bool {
    let mut tags = value.split('-');
    let first = tags.next().unwrap_or("");
    (2..=8).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && tags.all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()))
}
fn patch_field<'a>(map: &'a Map<String, Value>, snake: &str, camel: &str) -> Option<&'a Value> {
    map.get(snake).or_else(|| map.get(camel))
}
fn bounded_integer(value: &Value, default: i64, max: i64) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .unwrap_or(default)
        .clamp(1, max)
}
fn merge_preferences(mut current: Value, patch: Value) -> Result<Value, String> {
    let input = patch
        .as_object()
        .ok_or("Preferences must be a JSON object")?;
    let out = current
        .as_object_mut()
        .ok_or("Preferences must be a JSON object")?;
    for (key, alias, allowed) in [
        ("theme", "theme", vec!["light", "dark", "hologram"]),
        ("hologram_tone", "hologramTone", vec!["light", "dark"]),
        ("ui_lang", "uiLang", LANGS.to_vec()),
        (
            "provider",
            "provider",
            vec![
                "auto",
                "openai",
                "anthropic",
                "gemini",
                "xai",
                "claude_cli",
                "grok_cli",
                "codex_cli",
                "demo",
            ],
        ),
        (
            "purpose",
            "purpose",
            vec![
                "general",
                "technical",
                "ui",
                "subtitles",
                "game",
                "legal",
                "academic",
                "business",
                "custom",
            ],
        ),
        ("source_type", "sourceType", vec!["file", "folder", "zip"]),
        ("content_mode", "contentMode", vec!["document", "game"]),
    ] {
        if let Some(value) = patch_field(input, key, alias)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| allowed.contains(s))
        {
            out.insert(key.into(), json!(value));
        }
    }
    for (key, alias) in [
        ("ui_lang_follow", "uiLangFollow"),
        ("reduced_motion", "reducedMotion"),
    ] {
        if let Some(v) = patch_field(input, key, alias).and_then(Value::as_bool) {
            out.insert(key.into(), json!(v));
        }
    }
    for (key, alias, default, max) in [
        ("concurrency", "concurrency", 3, 16),
        ("per_provider", "perProvider", 1, 8),
    ] {
        if let Some(v) = patch_field(input, key, alias) {
            out.insert(key.into(), json!(bounded_integer(v, default, max)));
        }
    }
    if let Some(name) = input.get("project").and_then(Value::as_str) {
        out.insert("project".into(), json!(safe_project_name(name)));
    }
    for (key, alias) in [
        ("target_lang", "targetLang"),
        ("source_choice", "sourceChoice"),
    ] {
        if let Some(v) = patch_field(input, key, alias)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| language_code(s))
        {
            out.insert(key.into(), json!(v));
        }
    }
    if let Some(v) = patch_field(input, "target_other", "targetOther").and_then(Value::as_str) {
        out.insert(
            "target_other".into(),
            json!(v
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(40)
                .collect::<String>()),
        );
    }
    if let Some(models) =
        patch_field(input, "model_by_provider", "modelByProvider").and_then(Value::as_object)
    {
        let mut clean = out
            .get("model_by_provider")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (id, v) in models {
            if known_provider(id) {
                if let Some(model) = v.as_str().map(str::trim).filter(|s| safe_model(s)) {
                    clean.insert(id.clone(), json!(model));
                }
            }
        }
        out.insert("model_by_provider".into(), Value::Object(clean));
    }
    if let Some(rows) = input.get("chain").and_then(Value::as_array) {
        let mut seen = HashSet::new();
        let mut clean = Vec::new();
        for row in rows {
            if let Some(id) = row["id"]
                .as_str()
                .filter(|id| known_provider(id) && seen.insert((*id).to_owned()))
            {
                let model = row["model"].as_str().unwrap_or("").trim();
                let effort = row["effort"].as_str().unwrap_or("").trim();
                clean.push(json!({"id":id,"model":if safe_model(model){model}else{""},"effort":if ["","low","medium","high"].contains(&effort){effort}else{""},"enabled":row["enabled"].as_bool().unwrap_or(true)}));
            }
        }
        out.insert("chain".into(), json!(clean));
    }
    Ok(current)
}
fn read_preferences(path: &Path) -> Result<Value, String> {
    if !path
        .try_exists()
        .map_err(|_| "Cannot inspect preferences")?
    {
        reject_links(path)?;
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(&read_bytes(path, JSON_CAP)?)
        .map_err(|_| "Preferences are corrupt; original file was preserved")?;
    if !value.is_object() {
        return Err("Preferences must be an object; original file was preserved".into());
    }
    merge_preferences(json!({}), value)
}
fn project_names(root: &Path) -> Result<Vec<String>, String> {
    reject_links(root)?;
    if !root.try_exists().map_err(|_| "Cannot inspect projects")? {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| "Cannot list projects")? {
        let entry = entry.map_err(|_| "Cannot read project directory")?;
        reject_links(&entry.path())?;
        if entry.path().is_dir() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Project name is not Unicode")?;
            let glossary = entry.path().join("glossary.json");
            reject_links(&glossary)?;
            if glossary.is_file()
                || fs::read_dir(entry.path())
                    .map_err(|_| "Cannot inspect project directory")?
                    .next()
                    .is_none()
            {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}
fn read_glossary(path: &Path) -> Result<Vec<Term>, String> {
    reject_links(path)?;
    if !path.try_exists().map_err(|_| "Cannot inspect glossary")? {
        return Ok(vec![]);
    }
    let text = String::from_utf8(read_bytes(path, JSON_CAP)?)
        .map_err(|_| "Glossary is not UTF-8; original file was preserved")?;
    parse_glossary_json(&text)
        .map_err(|_| "Glossary is corrupt; original file was preserved".into())
}
fn parse_prompt(text: &str) -> Result<(u64, String), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if let Some(first) = lines.next() {
        if let Some(version) = first.strip_prefix("version:") {
            let version = version
                .trim()
                .parse::<u64>()
                .map_err(|_| "Custom prompt version is corrupt; original file was preserved")?;
            if lines.next().map(str::trim) != Some("---") {
                return Err("Custom prompt header is corrupt; original file was preserved".into());
            }
            return Ok((version, lines.collect::<Vec<_>>().join("\n").trim().into()));
        }
    }
    Ok((0, text.trim().into()))
}
fn read_custom(path: &Path) -> Result<(u64, String), String> {
    reject_links(path)?;
    if !path
        .try_exists()
        .map_err(|_| "Cannot inspect custom prompt")?
    {
        return Ok((0, String::new()));
    }
    parse_prompt(
        &String::from_utf8(read_bytes(path, PROMPT_CAP)?)
            .map_err(|_| "Custom prompt is not UTF-8; original file was preserved")?,
    )
}
fn patch_provider(config: &mut ProviderConfig, input: &Map<String, Value>) -> Result<(), String> {
    for (field, alias) in [
        ("model", "model"),
        ("effort", "effort"),
        ("api_key", "apiKey"),
        ("base_url", "baseUrl"),
        ("cli_path", "cliPath"),
    ] {
        if let Some(value) = patch_field(input, field, alias) {
            let value = value
                .as_str()
                .ok_or("Provider values must be strings")?
                .trim();
            match field {
                "model" if safe_model(value) => config.model = value.into(),
                "effort" if ["", "low", "medium", "high"].contains(&value) => {
                    config.effort = value.into()
                }
                "api_key"
                    if (value.is_empty() || !config.id.ends_with("_cli"))
                        && value.len() <= 4096
                        && !value.chars().any(char::is_control) =>
                {
                    config.api_key = value.into()
                }
                "base_url" if value.is_empty() || !config.id.ends_with("_cli") => {
                    if !value.is_empty() {
                        let endpoint = validate_endpoint(value, &config.id).map_err(|_| {
                            "Only allowlisted public developer HTTPS endpoints are permitted"
                        })?;
                        if endpoint.query().is_some() {
                            return Err("Developer API base URLs cannot contain query credentials or parameters".into());
                        }
                    }
                    config.base_url = value.into();
                }
                "cli_path"
                    if value.is_empty()
                        || (config.id.ends_with("_cli")
                            && Path::new(value).is_absolute()
                            && value.len() <= 32767
                            && !value.chars().any(char::is_control)) =>
                {
                    config.cli_path = value.into()
                }
                _ => return Err("Invalid provider setting; no changes were saved".into()),
            }
        }
    }
    Ok(())
}
fn read_credentials(path: &Path) -> Result<BTreeMap<String, ProviderConfig>, String> {
    reject_links(path)?;
    if !path
        .try_exists()
        .map_err(|_| "Cannot inspect encrypted credentials")?
    {
        return Ok(BTreeMap::new());
    }
    let bytes = read_bytes(path, JSON_CAP)?;
    let cipher = bytes
        .strip_prefix(CREDENTIAL_MAGIC)
        .ok_or("Encrypted credentials have an invalid header; original file was preserved")?;
    let mut plain = unprotect(cipher)?;
    let parsed: Result<Value, _> = serde_json::from_slice(&plain);
    plain.fill(0);
    let parsed = parsed
        .map_err(|_| "Encrypted credential records are corrupt; original file was preserved")?;
    let rows = parsed
        .as_array()
        .ok_or("Encrypted credential records must be an array")?;
    let mut output = BTreeMap::new();
    for row in rows {
        let input = row
            .as_object()
            .ok_or("Encrypted provider record is invalid")?;
        let id = input
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| IDS.contains(id))
            .ok_or("Encrypted provider id is invalid")?;
        if output.contains_key(id) {
            return Err("Encrypted provider ids are duplicated".into());
        }
        let mut config = default_config(id);
        patch_provider(&mut config, input)?;
        output.insert(id.into(), config);
    }
    Ok(output)
}
fn write_credentials(path: &Path, rows: &BTreeMap<String, ProviderConfig>) -> Result<(), String> {
    let records:Vec<_>=rows.values().map(|c|json!({"id":c.id,"model":c.model,"effort":c.effort,"apiKey":c.api_key,"baseUrl":c.base_url,"cliPath":c.cli_path})).collect();
    let mut plain =
        serde_json::to_vec(&records).map_err(|_| "Cannot encode encrypted provider records")?;
    let result = protect(&plain);
    plain.fill(0);
    let cipher = result?;
    let mut bytes = CREDENTIAL_MAGIC.to_vec();
    bytes.extend_from_slice(&cipher);
    atomic_write(path, &bytes, true)
}
#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if bytes.len() > u32::MAX as usize {
        return Err("Credential data is too large".into());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let entropy = b"Versora/native-user-credentials/v1";
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // No CRYPTPROTECT_LOCAL_MACHINE: encryption is bound to the current Windows user.
    let success = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(if encrypt {
            "Windows DPAPI could not encrypt credentials"
        } else {
            "Windows DPAPI could not unlock credentials for this user; original file was preserved"
        }
        .into());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        if !encrypt {
            std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        }
        LocalFree(output.pbData as _);
    }
    Ok(result)
}
#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, true)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, false)
}
#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("Credential storage requires Windows user-scope DPAPI".into())
}
#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("Credential storage requires Windows user-scope DPAPI".into())
}
fn json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = serde_json::to_vec_pretty(value).map_err(|_| "Cannot encode user settings")?;
    out.push(b'\n');
    Ok(out)
}
fn reject_links(path: &Path) -> Result<(), String> {
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(metadata) => {
                let mut link = metadata.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    link |= metadata.file_attributes() & 0x400 != 0;
                }
                if link {
                    return Err(
                        "Symbolic links and Windows reparse points are refused in user-data paths"
                            .into(),
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect user-data path".into()),
        }
    }
    Ok(())
}
fn ensure_dir(path: &Path) -> Result<(), String> {
    reject_links(path)?;
    fs::create_dir_all(path).map_err(|_| "Cannot create user-data directory")?;
    reject_links(path)
}
fn read_bytes(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    reject_links(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot read user-data file")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect user-data file")?;
    if !metadata.is_file() || metadata.len() > cap {
        return Err("User-data file is not regular or exceeds its size limit".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("User-data file is a reparse point".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read user-data file")?;
    if bytes.len() as u64 > cap {
        return Err("User-data file exceeds its size limit".into());
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("User-data file has no parent")?;
    ensure_dir(parent)?;
    reject_links(path)?;
    let temp = parent.join(format!(
        ".versora-write-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "Cannot reserve atomic user-data write")?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| "Cannot write user-data file")?;
        file.sync_all().map_err(|_| "Cannot flush user-data file")?;
        drop(file);
        reject_links(path)?;
        atomic_move(&temp, path, replace)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
#[cfg(windows)]
fn atomic_move(from: &Path, to: &Path, replace: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8 | if replace { 1 } else { 0 }) } == 0 {
        Err("Atomic user-data replacement failed; destination was preserved".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn atomic_move(from: &Path, to: &Path, replace: bool) -> Result<(), String> {
    if !replace {
        fs::hard_link(from, to).map_err(|_| "Destination already exists or atomic write failed")?;
        fs::remove_file(from).map_err(|_| "Cannot remove completed temporary write")?;
    } else {
        fs::rename(from, to).map_err(|_| "Atomic user-data replacement failed")?;
    }
    Ok(())
}
fn read_legacy_env(path: &Path) -> Result<BTreeMap<String, String>, String> {
    reject_links(path)?;
    if !path
        .try_exists()
        .map_err(|_| "Cannot inspect legacy .env")?
    {
        return Ok(BTreeMap::new());
    }
    let text =
        String::from_utf8(read_bytes(path, 64 * 1024)?).map_err(|_| "Legacy .env is not UTF-8")?;
    let mut result = BTreeMap::new();
    for line in text.strip_prefix('\u{feff}').unwrap_or(&text).lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let mut value = value.trim();
        if !IDS.iter().any(|id| {
            let prefix = id.trim_end_matches("_cli").to_ascii_uppercase();
            [
                format!("{prefix}_API_KEY"),
                format!("{prefix}_MODEL"),
                format!("{prefix}_BASE_URL"),
                format!("{prefix}_CLI_PATH"),
            ]
            .contains(&name.to_owned())
        }) && name != "DEFAULT_PROVIDER"
        {
            continue;
        }
        if value.starts_with(['\'', '"']) {
            let quote = value.chars().next().unwrap();
            if value.len() < 2 || !value.ends_with(quote) {
                return Err("A recognized legacy .env value has unmatched quotes".into());
            }
            value = &value[1..value.len() - 1];
        } else if let Some((head, _)) = value.split_once(" #") {
            value = head.trim_end();
        }
        result.insert(name.into(), value.into());
    }
    Ok(result)
}
fn legacy_config(
    id: &str,
    env: &BTreeMap<String, String>,
) -> Result<Option<ProviderConfig>, String> {
    let prefix = id.trim_end_matches("_cli").to_ascii_uppercase();
    let fields = if id.ends_with("_cli") {
        vec![
            ("cliPath", format!("{prefix}_CLI_PATH")),
            ("model", format!("{prefix}_MODEL")),
        ]
    } else {
        vec![
            ("apiKey", format!("{prefix}_API_KEY")),
            ("model", format!("{prefix}_MODEL")),
            ("baseUrl", format!("{prefix}_BASE_URL")),
        ]
    };
    let mut patch = Map::new();
    for (field, key) in fields {
        if let Some(value) = env.get(&key).filter(|v| !v.is_empty()) {
            patch.insert(field.into(), json!(value));
        }
    }
    if patch.is_empty() {
        return Ok(None);
    }
    let mut config = default_config(id);
    patch_provider(&mut config, &patch)?;
    Ok(Some(config))
}
fn safe_filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|s| matches!(s, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
    });
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.ends_with([' ', '.'])
        && !device
        && name.encode_utf16().count() <= 255
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}
fn collect_outputs(
    root: &Path,
    relative: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
    total: &mut u64,
) -> Result<(), String> {
    reject_links(root)?;
    if !root
        .try_exists()
        .map_err(|_| "Cannot inspect legacy outputs")?
    {
        return Ok(());
    }
    if !root.is_dir() {
        return Err("Legacy outputs must be a directory".into());
    }
    for entry in fs::read_dir(root).map_err(|_| "Cannot list legacy outputs")? {
        let entry = entry.map_err(|_| "Cannot inspect legacy output")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Legacy output name is not Unicode")?;
        if !safe_filename(&name) {
            return Err("Legacy output has an unsafe Windows name".into());
        }
        let path = entry.path();
        reject_links(&path)?;
        let next = relative.join(name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| "Cannot inspect legacy output")?;
        if metadata.is_dir() {
            collect_outputs(&path, &next, out, total)?;
        } else if metadata.is_file() {
            if metadata.len() > 128 * 1024 * 1024 {
                return Err("Legacy output exceeds 128 MiB".into());
            }
            *total = total
                .checked_add(metadata.len())
                .ok_or("Legacy import size overflow")?;
            if *total > 512 * 1024 * 1024 || out.len() >= 4000 {
                return Err("Legacy output import exceeds 512 MiB or 4000 files".into());
            }
            out.push((path, next));
        } else {
            return Err("Legacy output is not a regular file".into());
        }
    }
    Ok(())
}
