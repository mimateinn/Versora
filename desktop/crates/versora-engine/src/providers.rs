//! Direct, cancellable developer API transports. No Python SDK or browser endpoint.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

pub type CancelToken = Arc<AtomicBool>;
pub const OUTPUT_CAP: usize = 4 * 1024 * 1024;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub id: String,
    pub model: String,
    pub effort: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub base_url: String,
    pub cli_path: String,
}
impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("model", &self.model)
            .field("key_present", &!self.api_key.is_empty())
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderError {
    pub kind: String,
    pub detail: String,
    pub provider: String,
    pub retry_after: Option<f64>,
}
impl ProviderError {
    pub fn new(kind: &str, detail: impl AsRef<str>, provider: &str) -> Self {
        Self {
            kind: kind.into(),
            detail: redact(detail.as_ref(), &[]).chars().take(240).collect(),
            provider: provider.into(),
            retry_after: None,
        }
    }
    pub fn cancelled() -> Self {
        Self::new("cancelled", "Translation stopped", "")
    }
}
impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.detail)
    }
}
impl std::error::Error for ProviderError {}

pub fn is_secret_name(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.starts_with("NEXT_PUBLIC_")
        || [
            "_API_KEY",
            "_SECRET",
            "_SECRET_KEY",
            "_TOKEN",
            "_ACCESS_KEY",
            "_PASSWORD",
        ]
        .iter()
        .any(|s| n.ends_with(s))
        || matches!(
            n.as_str(),
            "API_KEY"
                | "SECRET"
                | "SECRET_KEY"
                | "TOKEN"
                | "ACCESS_KEY"
                | "PASSWORD"
                | "AWS_ACCESS_KEY_ID"
                | "AWS_SECRET_ACCESS_KEY"
                | "GOOGLE_APPLICATION_CREDENTIALS"
                | "AZURE_CLIENT_SECRET"
                | "OPENAI_KEY"
                | "ANTHROPIC_AUTH_TOKEN"
        )
}
pub fn redact(text: &str, extra: &[&str]) -> String {
    let mut out = text.to_owned();
    for secret in extra.iter().copied().map(str::to_owned).chain(
        std::env::vars()
            .filter(|(k, _)| is_secret_name(k))
            .map(|(_, v)| v),
    ) {
        if !secret.is_empty() {
            out = out.replace(&secret, "****");
        }
    }
    static RX: OnceLock<regex::Regex> = OnceLock::new();
    RX.get_or_init(|| {
        regex::Regex::new(r"(sk-|rk-|xai-|gsk_|AIza)[A-Za-z0-9_-]{8,}").expect("constant regex")
    })
    .replace_all(&out, "****")
    .into_owned()
}
pub fn check_cancel(cancel: &CancelToken) -> Result<(), ProviderError> {
    if cancel.load(Ordering::Acquire) {
        Err(ProviderError::cancelled())
    } else {
        Ok(())
    }
}
pub async fn cancellable_wait(
    duration: Duration,
    cancel: &CancelToken,
) -> Result<(), ProviderError> {
    let until = Instant::now() + duration;
    loop {
        check_cancel(cancel)?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        tokio::time::sleep(left.min(Duration::from_millis(25))).await;
    }
}
pub async fn cancelled(cancel: &CancelToken) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub fn default_model(id: &str) -> &str {
    match id {
        "openai" => "gpt-4.1-mini",
        "anthropic" => "claude-haiku-4-5",
        "gemini" => "gemini-2.5-flash",
        "xai" => "grok-3-mini",
        _ => "",
    }
}
pub fn default_base_url(id: &str) -> &str {
    match id {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com",
        "gemini" => "https://generativelanguage.googleapis.com",
        "xai" => "https://api.x.ai/v1",
        _ => "",
    }
}
pub fn kind_for_status(status: u16) -> &'static str {
    match status {
        401 | 403 => "auth",
        408 | 504 => "timeout",
        429 | 500..=599 => "limit",
        _ => "spawn",
    }
}
pub fn backoff(attempt: usize, retry_after: Option<f64>) -> Option<Duration> {
    if retry_after.is_some_and(|wait| !wait.is_finite()) {
        return None;
    }
    let requested = retry_after.unwrap_or(0.0).max(0.0);
    if !requested.is_finite() || requested > 30.0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        (2_f64.powi(attempt.min(5) as i32)).min(30.0).max(requested),
    ))
}

pub struct RequestSpec {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}
pub fn request_spec(
    config: &ProviderConfig,
    system: &str,
    user: &str,
) -> Result<RequestSpec, ProviderError> {
    let id = config.id.as_str();
    if config.api_key.trim().is_empty() {
        return Err(ProviderError::new("auth", "API key is missing", id));
    }
    let model = if config.model.trim().is_empty() {
        default_model(id)
    } else {
        config.model.trim()
    };
    if model.is_empty() || model.chars().count() > 256 || model.contains(['\r', '\n']) {
        return Err(ProviderError::new("spawn", "Invalid model identifier", id));
    }
    let base = if config.base_url.trim().is_empty() {
        default_base_url(id)
    } else {
        config.base_url.trim()
    }
    .trim_end_matches('/');
    let spec = match id {
        "openai" | "xai" => RequestSpec {
            url: format!("{base}/chat/completions"),
            headers: vec![("authorization".into(), format!("Bearer {}", config.api_key))],
            body: json!({"model":model,"messages":[{"role":"system","content":system},{"role":"user","content":user}]}),
        },
        "anthropic" => RequestSpec {
            url: format!("{base}/v1/messages"),
            headers: vec![
                ("x-api-key".into(), config.api_key.clone()),
                ("anthropic-version".into(), "2023-06-01".into()),
            ],
            body: json!({"model":model,"max_tokens":8192,"system":system,"messages":[{"role":"user","content":user}]}),
        },
        "gemini" => {
            if !model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return Err(ProviderError::new(
                    "spawn",
                    "Invalid Gemini model identifier",
                    id,
                ));
            }
            RequestSpec {
                url: format!("{base}/v1beta/models/{model}:generateContent"),
                headers: vec![("x-goog-api-key".into(), config.api_key.clone())],
                body: json!({"systemInstruction":{"parts":[{"text":system}]},"contents":[{"role":"user","parts":[{"text":user}]}]}),
            }
        }
        _ => return Err(ProviderError::new("spawn", "Unknown API provider", id)),
    };
    Ok(spec)
}

pub fn parse_response(id: &str, data: &Value) -> Result<String, ProviderError> {
    let fail = |kind, detail| ProviderError::new(kind, detail, id);
    let text = match id {
        "openai" | "xai" => {
            let choice = &data["choices"][0];
            match choice["finish_reason"].as_str().unwrap_or("") {
                "length" => return Err(fail("truncated", "finish_reason=length")),
                "content_filter" => return Err(fail("refused", "finish_reason=content_filter")),
                _ => {}
            }
            if choice["message"]["refusal"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            {
                return Err(fail("refused", "Provider returned a refusal"));
            }
            choice["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        }
        "anthropic" => {
            match data["stop_reason"].as_str().unwrap_or("") {
                "max_tokens" => return Err(fail("truncated", "stop_reason=max_tokens")),
                "refusal" => return Err(fail("refused", "stop_reason=refusal")),
                _ => {}
            }
            data["content"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|b| b["type"] == "text")
                        .filter_map(|b| b["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        }
        "gemini" => {
            if data["promptFeedback"]["blockReason"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            {
                return Err(fail("refused", "Prompt blocked by provider"));
            }
            let candidate = &data["candidates"][0];
            match candidate["finishReason"].as_str().unwrap_or("") {
                "MAX_TOKENS" => return Err(fail("truncated", "finishReason=MAX_TOKENS")),
                "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" => {
                    return Err(fail("refused", "Content blocked by provider"))
                }
                _ => {}
            }
            candidate["content"]["parts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|b| b["thought"] != true)
                        .filter_map(|b| b["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        }
        _ => return Err(fail("spawn", "Unknown API provider")),
    };
    if text.trim().is_empty() {
        Err(fail("empty", "No text content"))
    } else {
        Ok(text.trim().into())
    }
}

fn allowed_host(host: &str) -> bool {
    let blocked = [
        "chat.openai.com",
        "chatgpt.com",
        "www.chatgpt.com",
        "claude.ai",
        "www.claude.ai",
        "gemini.google.com",
        "aistudio.google.com",
        "grok.x.ai",
        "grok.com",
        "www.grok.com",
        "cli-chat-proxy.grok.com",
        "auth.x.ai",
        "accounts.x.ai",
        "x.com",
        "twitter.com",
    ];
    if blocked.contains(&host) {
        return false;
    }
    [
        "api.openai.com",
        "api.anthropic.com",
        "generativelanguage.googleapis.com",
        "api.groq.com",
        "api.deepseek.com",
        "api.x.ai",
        "api.openrouter.ai",
    ]
    .contains(&host)
        || host == "openai.azure.com"
        || host.ends_with(".openai.azure.com")
        || std::env::var("ALLOWED_API_HOSTS")
            .unwrap_or_default()
            .split(',')
            .any(|h| h.trim().trim_end_matches('.').eq_ignore_ascii_case(host))
}
pub fn validate_endpoint(url: &str, id: &str) -> Result<reqwest::Url, ProviderError> {
    let fail = || {
        ProviderError::new(
            "spawn",
            "Only allowlisted public developer HTTPS endpoints are permitted",
            id,
        )
    };
    let parsed = reqwest::Url::parse(url).map_err(|_| fail())?;
    let host = parsed.host_str().ok_or_else(fail)?.to_ascii_lowercase();
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || host.parse::<IpAddr>().is_ok()
        || !allowed_host(host.trim_end_matches('.'))
    {
        return Err(fail());
    }
    Ok(parsed)
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_unspecified()
                || v.is_multicast()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            v.to_ipv4_mapped()
                .map(|v| public_ip(IpAddr::V4(v)))
                .unwrap_or_else(|| {
                    s[0] & 0xe000 == 0x2000
                        && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                        && (s[0] != 0x2002
                            || public_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                                (s[1] >> 8) as u8,
                                s[1] as u8,
                                (s[2] >> 8) as u8,
                                s[2] as u8,
                            ))))
                })
        }
    }
}
async fn secure_client(url: &reqwest::Url, id: &str) -> Result<reqwest::Client, ProviderError> {
    let host = url.host_str().expect("validated endpoint");
    let addresses: Vec<SocketAddr> =
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443)))
            .await
            .map_err(|_| {
                ProviderError::new("spawn", "Developer API hostname could not be resolved", id)
            })?
            .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(ProviderError::new(
            "spawn",
            "Developer API hostname resolved to a non-public address",
            id,
        ));
    }
    // Pin the validated DNS answer. Redirects and environment proxies are disabled.
    type Clients = Mutex<HashMap<String, reqwest::Client>>;
    static CLIENTS: OnceLock<Clients> = OnceLock::new();
    let key = format!("{host}:{addresses:?}");
    let mut cache = CLIENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(client) = cache.get(&key) {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30))
        .timeout(CALL_TIMEOUT)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| ProviderError::new("spawn", "HTTPS client initialization failed", id))?;
    if cache.len() > 16 {
        cache.clear();
    }
    cache.insert(key, client.clone());
    Ok(client)
}
async fn send_http(
    client: &reqwest::Client,
    config: &ProviderConfig,
    spec: RequestSpec,
    cancel: &CancelToken,
) -> Result<String, ProviderError> {
    let mut request = client.post(&spec.url).json(&spec.body);
    for (name, value) in spec.headers {
        request = request.header(name, value);
    }
    let mut response = request.send().await.map_err(|e| {
        ProviderError::new(
            if e.is_timeout() { "timeout" } else { "spawn" },
            "Developer API request failed",
            &config.id,
        )
    })?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite())
        .map(|s| s.max(0.0));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        ProviderError::new(
            if e.is_timeout() { "timeout" } else { "spawn" },
            "Developer API response interrupted",
            &config.id,
        )
    })? {
        check_cancel(cancel)?;
        if bytes.len() + chunk.len() > OUTPUT_CAP {
            return Err(ProviderError::new(
                "truncated",
                "API output exceeded 4 MiB",
                &config.id,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !(200..300).contains(&status) {
        // Never forward response JSON or request headers: provider errors can echo document/key data.
        let mut error = ProviderError::new(
            kind_for_status(status),
            format!("HTTP {status}"),
            &config.id,
        );
        error.retry_after = retry_after;
        return Err(error);
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ProviderError::new("empty", "Provider response was not JSON", &config.id))?;
    parse_response(&config.id, &data)
}
async fn send_cancellable(
    client: &reqwest::Client,
    config: &ProviderConfig,
    spec: RequestSpec,
    cancel: &CancelToken,
) -> Result<String, ProviderError> {
    tokio::select! {biased;_=cancelled(cancel)=>Err(ProviderError::cancelled()),response=send_http(client,config,spec,cancel)=>response}
}
pub async fn complete(
    config: &ProviderConfig,
    system: &str,
    user: &str,
    cancel: &CancelToken,
) -> Result<String, ProviderError> {
    check_cancel(cancel)?;
    if config.id.ends_with("_cli") {
        return crate::cli::complete(config, system, user, cancel).await;
    }
    if config.id == "demo" {
        // Explicit opt-in preview transport only. It tags text, never claims translation.
        if !["VERSORA_DEMO", "SFTS_DEMO"]
            .iter()
            .any(|name| std::env::var(name).as_deref() == Ok("1"))
        {
            return Err(ProviderError::new(
                "auth",
                "Offline demo must be explicitly enabled",
                "demo",
            ));
        }
        cancellable_wait(Duration::from_millis(300), cancel).await?;
        let target = user
            .lines()
            .find_map(|l| l.strip_prefix("Target language:"))
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("xx");
        let rx = regex::Regex::new(r"^(\d+)\.\s?(.*)$").expect("constant regex");
        return Ok(user
            .split('\n')
            .filter_map(|line| rx.captures(line))
            .map(|c| format!("{}. [{}] {}", &c[1], target, &c[2]))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let request = request_spec(config, system, user)?;
    let url = validate_endpoint(&request.url, &config.id)?;
    let work = async {
        let client = secure_client(&url, &config.id).await?;
        send_cancellable(&client, config, request, cancel).await
    };
    tokio::select! { biased; _ = cancelled(cancel) => Err(ProviderError::cancelled()), result = tokio::time::timeout(CALL_TIMEOUT, work) => result.unwrap_or_else(|_| Err(ProviderError::new("timeout", "No API response within 120 seconds", &config.id))) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub id: String,
    pub present: bool,
    pub version: String,
    pub version_ok: bool,
    pub signed_in: Option<bool>,
    pub usable: bool,
    pub models: Vec<String>,
    pub detail: String,
    pub limitations: Vec<String>,
}
pub async fn probe(config: &ProviderConfig, fresh: bool, cancel: &CancelToken) -> ProviderStatus {
    if config.id.ends_with("_cli") {
        return crate::cli::probe(config, fresh, cancel).await;
    }
    let present = !config.api_key.trim().is_empty();
    let endpoint_ok = validate_endpoint(default_base_url(&config.id), &config.id).is_ok()
        && (config.base_url.is_empty() || validate_endpoint(&config.base_url, &config.id).is_ok());
    ProviderStatus {
        id: config.id.clone(),
        present,
        version: String::new(),
        version_ok: false,
        signed_in: None,
        usable: present && endpoint_ok,
        models: vec![],
        detail: if present && endpoint_ok {
            "Configured; live API credentials, credit and model compatibility have not been tested"
        } else {
            "Missing API key or invalid developer endpoint"
        }
        .into(),
        limitations: vec!["Live paid translation NOT_RUN".into()],
    }
}

struct GateSet {
    global: Arc<Semaphore>,
    per: Mutex<HashMap<String, Arc<Semaphore>>>,
    global_n: usize,
    per_n: usize,
}
static GATES: OnceLock<Mutex<Arc<GateSet>>> = OnceLock::new();
fn gates(global: usize, per: usize) -> Arc<GateSet> {
    let global = global.clamp(1, 16);
    let per = per.clamp(1, 8);
    let new = || {
        Arc::new(GateSet {
            global: Arc::new(Semaphore::new(global)),
            per: Mutex::new(HashMap::new()),
            global_n: global,
            per_n: per,
        })
    };
    let mut hit = GATES
        .get_or_init(|| Mutex::new(new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if hit.global_n != global || hit.per_n != per {
        *hit = new();
    }
    hit.clone()
}
#[derive(Clone, Debug, Serialize)]
pub struct ProviderHealth {
    pub kind: Option<String>,
    pub retry_after: Option<f64>,
}
#[derive(Clone)]
pub struct ProviderRuntime {
    gates: Arc<GateSet>,
    health: Arc<Mutex<HashMap<String, ProviderHealth>>>,
    usage: Arc<Mutex<HashSet<String>>>,
}
pub trait ProviderTransport: Send + Sync {
    fn complete<'a>(
        &'a self,
        config: &'a ProviderConfig,
        system: &'a str,
        user: &'a str,
        cancel: &'a CancelToken,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ProviderError>> + Send + 'a>,
    >;
}
struct NativeTransport;
impl ProviderTransport for NativeTransport {
    fn complete<'a>(
        &'a self,
        config: &'a ProviderConfig,
        system: &'a str,
        user: &'a str,
        cancel: &'a CancelToken,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ProviderError>> + Send + 'a>,
    > {
        Box::pin(complete(config, system, user, cancel))
    }
}
impl ProviderRuntime {
    pub fn new(global: usize, per: usize) -> Self {
        Self {
            gates: gates(global, per),
            health: Arc::new(Mutex::new(HashMap::new())),
            usage: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    /// A file operation shares concurrency and health while retaining its own provenance.
    pub fn for_operation(&self) -> Self {
        Self {
            gates: self.gates.clone(),
            health: self.health.clone(),
            usage: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    pub fn used_demo(&self) -> bool {
        self.usage
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains("demo")
    }
    pub fn health(&self) -> HashMap<String, ProviderHealth> {
        self.health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub async fn call(
        &self,
        configs: &[ProviderConfig],
        system: &str,
        user: &str,
        cancel: &CancelToken,
    ) -> Result<String, ProviderError> {
        self.call_with(&NativeTransport, configs, system, user, cancel)
            .await
    }
    pub async fn call_with(
        &self,
        transport: &dyn ProviderTransport,
        configs: &[ProviderConfig],
        system: &str,
        user: &str,
        cancel: &CancelToken,
    ) -> Result<String, ProviderError> {
        let mut last = ProviderError::new(
            "auth",
            "No translator is configured; add one in Settings",
            "",
        );
        for config in configs {
            for attempt in 0..=2 {
                check_cancel(cancel)?;
                let sem = self
                    .gates
                    .per
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .entry(config.id.clone())
                    .or_insert_with(|| Arc::new(Semaphore::new(self.gates.per_n)))
                    .clone();
                let permits = async {
                    let p = sem.acquire_owned().await.map_err(|_| {
                        ProviderError::new("spawn", "Provider gate closed", &config.id)
                    })?;
                    let g = self
                        .gates
                        .global
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| {
                            ProviderError::new("spawn", "Global gate closed", &config.id)
                        })?;
                    Ok::<_, ProviderError>((p, g))
                };
                let guard = tokio::select! { biased; _ = cancelled(cancel) => return Err(ProviderError::cancelled()), p = permits => p? };
                let result = transport.complete(config, system, user, cancel).await;
                drop(guard);
                match result {
                    Ok(text) => {
                        self.usage
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(config.id.clone());
                        self.health
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(
                                config.id.clone(),
                                ProviderHealth {
                                    kind: None,
                                    retry_after: None,
                                },
                            );
                        return Ok(text);
                    }
                    Err(error) if error.kind == "cancelled" || error.kind == "truncated" => {
                        self.health
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(
                                config.id.clone(),
                                ProviderHealth {
                                    kind: Some(error.kind.clone()),
                                    retry_after: error.retry_after,
                                },
                            );
                        return Err(error);
                    }
                    Err(error) => {
                        let wait = if attempt < 2
                            && matches!(error.kind.as_str(), "limit" | "timeout" | "empty")
                        {
                            backoff(attempt, error.retry_after)
                        } else {
                            None
                        };
                        self.health
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(
                                config.id.clone(),
                                ProviderHealth {
                                    kind: Some(error.kind.clone()),
                                    retry_after: wait.map(|w| w.as_secs_f64()),
                                },
                            );
                        last = error;
                        match wait {
                            Some(wait) => cancellable_wait(wait, cancel).await?,
                            None => break,
                        }
                    }
                }
            }
        }
        Err(last)
    }
    pub async fn translate_texts(
        &self,
        texts: &[String],
        options: &crate::translate::TranslationOptions,
        configs: &[ProviderConfig],
        system: &str,
        cancel: &CancelToken,
        progress: Option<crate::translate::Progress>,
    ) -> Result<Vec<String>, ProviderError> {
        crate::translate::translate_using(self, texts, options, configs, system, cancel, progress)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read_request(socket: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = socket.read(&mut buf).unwrap();
            assert_ne!(n, 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..index]);
                let length = header
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= index + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }
    #[tokio::test]
    async fn direct_http_protocol() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.contains("POST /chat/completions"));
            assert!(request.contains("Bearer test-key"));
            assert!(request.contains("Hello"));
            let body =
                r#"{"choices":[{"message":{"content":"1. Bonjour"},"finish_reason":"stop"}]}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let config = ProviderConfig {
            id: "openai".into(),
            api_key: "test-key".into(),
            base_url: format!("http://{address}"),
            ..Default::default()
        };
        let spec = request_spec(&config, "system", "1. Hello").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            send_http(&client, &config, spec, &cancel).await.unwrap(),
            "1. Bonjour"
        );
        handle.join().unwrap();
        // Production validation deliberately refuses this test-only loopback endpoint.
        assert!(validate_endpoint(&config.base_url, "openai").is_err());
    }
    #[tokio::test]
    async fn cancellation_interrupts_pending_http_body() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let _ = read_request(&mut socket);
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nContent-Type: application/json\r\n\r\n{{").unwrap();
            let _ = started_tx.send(());
            // Local cancellation cannot recall a request already received by a provider.
            finish_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        let config = ProviderConfig {
            id: "openai".into(),
            api_key: "test-key".into(),
            base_url: format!("http://{address}"),
            ..Default::default()
        };
        let spec = request_spec(&config, "system", "user").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        let task = tokio::spawn(async move {
            started_rx.await.unwrap();
            trigger.store(true, Ordering::Release);
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            send_cancellable(&client, &config, spec, &cancel),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.kind, "cancelled");
        task.await.unwrap();
        finish_tx.send(()).unwrap();
        handle.join().unwrap();
    }
    #[test]
    fn dns_address_policy() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "100.64.0.1",
            "192.0.0.2",
            "192.0.2.1",
            "198.18.0.1",
            "169.254.169.254",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
