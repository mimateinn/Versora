//! Suite contract v1 metadata discovery. This is deliberately NOT an installer/downloader.
//! GitHub metadata and checksums do not authenticate a publisher. Until a reviewed Tauri
//! trust anchor and tested native replacement/recovery adapter exist, execution is blocked.

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const OFFICIAL_RELEASE_URL: &str = "https://github.com/mimateinn/Versora/releases";
pub const CAPABILITY_REASON: &str = "Automatic download and installation are disabled: no reviewed publisher trust key or tested native installation/recovery adapter is configured. Use the official Versora release page.";
const METADATA_CAP: usize = 2 * 1024 * 1024;
const STATE_CAP: u64 = 256 * 1024;
const PACKAGE_CAP: u64 = 1024 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub policy: String,
    pub interval_hours: u16,
    pub channel: String,
    pub auto_download: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            policy: "startup".into(),
            interval_hours: 24,
            channel: "stable".into(),
            auto_download: false,
        }
    }
}

impl Preferences {
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.policy.as_str(), "manual" | "startup" | "periodic")
            || !matches!(self.channel.as_str(), "stable" | "preview")
            || !(1..=168).contains(&self.interval_hours)
        {
            return Err("Invalid update preferences: use manual/startup/periodic, stable/preview and 1–168 integer hours".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Candidate {
    pub version: String,
    pub tag: String,
    pub asset_name: String,
    pub asset_size: u64,
    pub release_url: String,
    pub platform: String,
    pub format: String,
}

impl Candidate {
    fn validate(&self, channel: &str) -> Result<Version, String> {
        let version = strict_version(&self.version)?;
        if !accepts_channel(&version, channel)
            || self.tag != format!("v{version}")
            || self.asset_name != package_name(&version)
            || self.release_url != release_url(&version)
            || self.platform != "win32-x64"
            || self.format != "nsis"
            || self.asset_size == 0
            || self.asset_size > PACKAGE_CAP
        {
            return Err("Persisted update candidate does not match the pinned Versora/channel/Windows NSIS contract".into());
        }
        Ok(version)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    schema: u8,
    preferences: Preferences,
    highest_healthy_version: Option<String>,
    candidate: Option<Candidate>,
    deferred: bool,
    last_check_at: Option<u64>,
    next_check_at: Option<u64>,
    failures: u8,
    in_progress: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            schema: 1,
            preferences: Preferences::default(),
            highest_healthy_version: None,
            candidate: None,
            deferred: false,
            last_check_at: None,
            next_check_at: None,
            failures: 0,
            in_progress: false,
        }
    }
}

impl Saved {
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.failures > 8 || (self.deferred && self.candidate.is_none()) {
            return Err("Update state has an invalid schema or lifecycle".into());
        }
        self.preferences.validate()?;
        if let Some(version) = &self.highest_healthy_version {
            strict_version(version)?;
        }
        if let Some(candidate) = &self.candidate {
            candidate.validate(&self.preferences.channel)?;
        }
        Ok(())
    }
}

struct Runtime {
    saved: Saved,
    status: &'static str,
    error: Option<String>,
}

pub struct UpdateService {
    directory: PathBuf,
    version: Version,
    runtime: Mutex<Runtime>,
    active: AtomicBool,
    cancellation: AtomicU64,
    started: AtomicBool,
}

impl UpdateService {
    pub fn new(data_dir: PathBuf, version: &str) -> Result<Self, String> {
        let version = strict_version(version)?;
        if !data_dir.is_absolute() {
            return Err("Update user-data directory must be absolute".into());
        }
        reject_links(&data_dir)?;
        let directory = data_dir.join("updates");
        reject_links(&directory)?;
        let mut saved = read_saved(&directory.join("state.json"))?;
        saved.validate()?;
        // A restarted interrupted metadata check is cancelled, never 'ready'. No package exists.
        let interrupted = saved.in_progress;
        saved.in_progress = false;
        clear_obsolete_candidate(&mut saved, &version)?;
        fs::create_dir_all(&directory).map_err(|_| "Cannot create update metadata directory")?;
        Ok(Self {
            directory,
            version,
            runtime: Mutex::new(Runtime {
                status: if interrupted {
                    "cancelled"
                } else if saved.candidate.is_some() {
                    "available"
                } else {
                    "idle"
                },
                saved,
                error: interrupted.then(|| {
                    "Previous metadata check was interrupted; no package was downloaded".into()
                }),
            }),
            active: AtomicBool::new(false),
            cancellation: AtomicU64::new(0),
            started: AtomicBool::new(false),
        })
    }

    pub fn official_release_url(&self) -> &'static str {
        OFFICIAL_RELEASE_URL
    }

    pub fn get_preferences(&self) -> Value {
        serde_json::to_value(
            &self
                .runtime
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .saved
                .preferences,
        )
        .expect("preferences serialize")
    }

    pub fn get_state(&self) -> Value {
        let state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        let saved = &state.saved;
        json!({ "status": state.status, "currentVersion": self.version.to_string(),
            "candidateVersion": saved.candidate.as_ref().map(|c| c.version.as_str()), "candidate": saved.candidate,
            "preferences": saved.preferences, "channel": saved.preferences.channel,
            "bytesReceived": 0, "totalBytes": 0, "error": state.error,
            "lastCheckAt": saved.last_check_at, "nextCheckAt": saved.next_check_at,
            "deferred": saved.deferred, "active": self.active.load(Ordering::Acquire),
            "highestHealthyVersion": saved.highest_healthy_version,
            "trust": false, "installCapabilities": false, "downloadCapability": false,
            "capabilityReason": CAPABILITY_REASON, "officialReleaseUrl": OFFICIAL_RELEASE_URL,
            "recoveryCapability": false, "packageDownloaded": false })
    }

    pub fn set_preferences(&self, patch: Value) -> Result<Value, String> {
        // Validate before signalling cancellation so bad renderer input has no side effects.
        patch_preferences(
            &self
                .runtime
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .saved
                .preferences,
            &patch,
        )?;
        if self.active.load(Ordering::Acquire) {
            self.cancellation.fetch_add(1, Ordering::AcqRel);
            return Err(
                "Update check is stopping; save these preferences again after it stops".into(),
            );
        }
        let operation = self.begin()?;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        let next = patch_preferences(&state.saved.preferences, &patch)?;
        let changed_channel = next.channel != state.saved.preferences.channel;
        let changed_policy = next.policy != state.saved.preferences.policy;
        let mut saved = state.saved.clone();
        saved.preferences = next;
        if changed_channel {
            saved.candidate = None;
            saved.deferred = false;
        }
        saved.next_check_at = if saved.preferences.policy == "periodic" {
            Some(now_ms().saturating_add(u64::from(saved.preferences.interval_hours) * 3_600_000))
        } else {
            None
        };
        self.save(&saved)?;
        state.saved = saved;
        state.status = if state.saved.candidate.is_some() {
            "available"
        } else {
            "idle"
        };
        state.error = None;
        if changed_policy || changed_channel {
            self.started.store(false, Ordering::Release);
        }
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    /// Manual checks bypass the schedule/backoff. One metadata operation per data directory.
    pub async fn check(&self) -> Result<Value, String> {
        if self.active.load(Ordering::Acquire) {
            return Ok(self.get_state());
        }
        let operation = self.begin()?;
        let generation = self.start_check()?;
        let metadata = tokio::select! {
            biased;
            _ = self.cancelled(generation) => Err("Update metadata check cancelled".to_owned()),
            result = tokio::time::timeout(TIMEOUT, self.fetch_metadata()) => result.unwrap_or_else(|_| Err("Update metadata check exceeded the 120-second deadline".into())),
        };
        let result = match metadata {
            Ok(releases) => self.finish_metadata(releases, now_ms(), generation),
            Err(error) => self.finish_error(error, now_ms(), generation),
        };
        drop(operation);
        result.map(|_| self.get_state())
    }

    pub fn cancel(&self) -> Result<Value, String> {
        self.cancellation.fetch_add(1, Ordering::AcqRel);
        if self.active.load(Ordering::Acquire) {
            self.runtime.lock().unwrap_or_else(|p| p.into_inner()).error =
                Some("Stopping metadata check".into());
            return Ok(self.get_state()); // active remains true until the request future has dropped.
        }
        let _operation = self.begin()?;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        state.status = "cancelled";
        state.error = None;
        drop(state);
        drop(_operation);
        Ok(self.get_state())
    }

    pub fn later(&self) -> Result<Value, String> {
        let operation = self.begin()?;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        if state.saved.candidate.is_none() {
            return Err("No available update metadata to defer".into());
        }
        let mut saved = state.saved.clone();
        saved.deferred = true;
        self.save(&saved)?;
        state.saved = saved;
        state.status = "available";
        state.error = None;
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    /// Caller combines native job/persistence truth with renderer unsaved-edit truth.
    /// This gate never executes, downloads, exits the app, or claims successful installation.
    pub fn request_install(&self, work_snapshot: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Work {
            busy: bool,
            unsaved: bool,
            pending_persistence: bool,
        }
        let work: Work =
            serde_json::from_value(work_snapshot).map_err(|_| "Invalid update work snapshot")?;
        if work.busy {
            return Err("Finish or cancel active translation jobs before updating".into());
        }
        if work.unsaved {
            return Err("Save or discard unsaved work before updating".into());
        }
        if work.pending_persistence {
            return Err(
                "Wait for settings and project data to finish saving before updating".into(),
            );
        }
        let operation = self.begin()?;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        state.error = Some(CAPABILITY_REASON.into());
        drop(state);
        drop(operation);
        let mut result = self.get_state();
        result["installBlocked"] = json!(true);
        result["appRemainsOpen"] = json!(true);
        Ok(result)
    }

    /// Explicit native/UI-ready ACK only. A version marker is not an installation/rollback claim.
    pub fn health_ack(&self) -> Result<Value, String> {
        let operation = self.begin()?;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        let mut saved = state.saved.clone();
        let highest = healthy_floor(&saved, &self.version)?;
        saved.highest_healthy_version = Some(highest.to_string());
        clear_obsolete_candidate(&mut saved, &self.version)?;
        self.save(&saved)?;
        state.saved = saved;
        if state.status == "available" && state.saved.candidate.is_none() {
            state.status = "idle";
        }
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    pub fn seconds_until_next_check(&self) -> Option<u64> {
        if self.active.load(Ordering::Acquire) {
            return None;
        }
        let state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        if state.saved.preferences.policy == "manual" {
            return None;
        }
        if !self.started.load(Ordering::Acquire) {
            return Some(0);
        }
        state
            .saved
            .next_check_at
            .map(|due| due.saturating_sub(now_ms()).div_ceil(1000))
    }

    pub fn is_check_due(&self) -> bool {
        self.seconds_until_next_check() == Some(0)
    }

    fn begin(&self) -> Result<Operation<'_>, String> {
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("An update operation is already active".into());
        }
        match lock_operation(&self.directory.join("operation.lock")) {
            Ok(file) => Ok(Operation {
                service: self,
                file: Some(file),
            }),
            Err(error) => {
                self.active.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    fn refresh(&self, state: &mut Runtime) -> Result<(), String> {
        let mut disk = read_saved(&self.directory.join("state.json"))?;
        disk.validate()?;
        // Another process or an older version can never lower a healthy marker in memory.
        if let Some(old) = &state.saved.highest_healthy_version {
            let old = strict_version(old)?;
            if disk
                .highest_healthy_version
                .as_ref()
                .map(|v| strict_version(v))
                .transpose()?
                .is_none_or(|v| v < old)
            {
                disk.highest_healthy_version = Some(old.to_string());
            }
        }
        clear_obsolete_candidate(&mut disk, &self.version)?;
        disk.in_progress = false;
        state.saved = disk;
        Ok(())
    }

    fn start_check(&self) -> Result<u64, String> {
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.refresh(&mut state)?;
        let mut saved = state.saved.clone();
        saved.in_progress = true;
        self.save(&saved)?;
        state.saved = saved;
        state.status = "checking";
        state.error = None;
        self.started.store(true, Ordering::Release);
        Ok(self.cancellation.load(Ordering::Acquire))
    }

    fn finish_metadata(&self, releases: Value, at: u64, generation: u64) -> Result<(), String> {
        if self.cancellation.load(Ordering::Acquire) != generation {
            return self.finish_error("Update metadata check cancelled".into(), at, generation);
        }
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        let candidate = match select_candidate(
            &releases,
            &state.saved.preferences.channel,
            &self.version.to_string(),
            state.saved.highest_healthy_version.as_deref(),
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                drop(state);
                return self.finish_error(error, at, generation);
            }
        };
        let mut saved = state.saved.clone();
        if saved.candidate != candidate {
            saved.deferred = false;
        }
        saved.candidate = candidate;
        saved.last_check_at = Some(at);
        saved.failures = 0;
        saved.in_progress = false;
        saved.next_check_at = periodic_due(&saved.preferences, at);
        self.save(&saved)?;
        state.saved = saved;
        state.status = if state.saved.candidate.is_some() {
            "available"
        } else {
            "idle"
        };
        state.error = None;
        Ok(())
    }

    fn finish_error(&self, error: String, at: u64, generation: u64) -> Result<(), String> {
        let cancelled = self.cancellation.load(Ordering::Acquire) != generation;
        let mut state = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        let mut saved = state.saved.clone();
        saved.in_progress = false;
        saved.last_check_at = Some(at);
        saved.failures = if cancelled {
            saved.failures
        } else {
            saved.failures.saturating_add(1).min(8)
        };
        saved.next_check_at = if cancelled {
            periodic_due(&saved.preferences, at)
        } else if saved.preferences.policy == "manual" {
            None
        } else {
            Some(at.saturating_add(backoff_seconds(saved.failures) * 1000))
        };
        let write = self.save(&saved);
        if write.is_ok() {
            state.saved = saved;
        }
        state.status = if cancelled { "cancelled" } else { "failed" };
        state.error = Some(write.err().unwrap_or_else(|| error.clone()));
        if cancelled {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn save(&self, saved: &Saved) -> Result<(), String> {
        saved.validate()?;
        let bytes =
            serde_json::to_vec_pretty(saved).map_err(|_| "Cannot serialize update state")?;
        atomic_write(&self.directory.join("state.json"), &bytes)
    }

    async fn cancelled(&self, generation: u64) {
        loop {
            if self.cancellation.load(Ordering::Acquire) != generation {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn fetch_metadata(&self) -> Result<Value, String> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(15))
            .user_agent(format!("Versora/{}", self.version))
            .build()
            .map_err(|_| "Cannot create HTTPS update metadata client")?;
        let mut all = Vec::new();
        let mut remaining = METADATA_CAP;
        for page in 1..=20 {
            let response = client
                .get(github_api_url(page)?)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
                .map_err(|_| "Official GitHub update metadata request failed")?;
            if !response.status().is_success() {
                return Err(format!(
                    "Official GitHub update metadata returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let bytes = bounded_response(response, remaining).await?;
            remaining -= bytes.len();
            let page_rows = parse_release_metadata(&bytes)?;
            let rows = page_rows
                .as_array()
                .ok_or("Official update metadata must be a release array")?;
            if rows.len() > 100 {
                return Err("Official update page exceeded the requested 100-release bound".into());
            }
            all.extend(rows.iter().cloned());
            if rows.len() < 100 {
                return Ok(Value::Array(all));
            }
        }
        Err(
            "Release history exceeds the bounded metadata check; use the official release page"
                .into(),
        )
    }

    #[cfg(test)]
    pub fn check_fixture(&self, releases: Value, at: u64) -> Result<Value, String> {
        let operation = self.begin()?;
        let generation = self.start_check()?;
        let result = self.finish_metadata(releases, at, generation);
        drop(operation);
        result.map(|_| self.get_state())
    }

    #[cfg(test)]
    pub async fn delayed_fixture(&self, releases: Value) -> Result<Value, String> {
        let operation = self.begin()?;
        let generation = self.start_check()?;
        tokio::select! { biased;
            _ = self.cancelled(generation) => { self.finish_error("cancelled".into(), now_ms(), generation)?; },
            _ = tokio::time::sleep(Duration::from_secs(10)) => { self.finish_metadata(releases, now_ms(), generation)?; },
        }
        drop(operation);
        Ok(self.get_state())
    }
}

struct Operation<'a> {
    service: &'a UpdateService,
    file: Option<File>,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut state = self
            .service
            .runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if state.status == "checking" {
            state.saved.in_progress = false;
            state.status = "cancelled";
            state.error = Some("Metadata operation interrupted; no package was downloaded".into());
            if let Err(error) = self.service.save(&state.saved) {
                state.status = "failed";
                state.error = Some(error);
            }
        }
        drop(state);
        drop(self.file.take());
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(self.service.directory.join("operation.lock"));
        }
        self.service.active.store(false, Ordering::Release);
    }
}

pub fn strict_version(text: &str) -> Result<Version, String> {
    if text.is_empty() || text.len() > 128 || !text.is_ascii() || text.contains('+') {
        return Err("Update version must be strict SemVer, at most 128 ASCII characters, without build metadata".into());
    }
    let version = Version::parse(text).map_err(|_| "Update version is not strict SemVer")?;
    if !version.build.is_empty() || version.to_string() != text {
        return Err("Update version is not canonical SemVer".into());
    }
    Ok(version)
}

pub fn github_api_url(page: usize) -> Result<String, String> {
    if !(1..=20).contains(&page) {
        return Err("Invalid fixed release metadata page".into());
    }
    Ok(format!(
        "https://api.github.com/repos/mimateinn/Versora/releases?per_page=100&page={page}"
    ))
}

pub fn parse_release_metadata(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > METADATA_CAP {
        return Err("Release metadata exceeds the 2 MiB bound".into());
    }
    let metadata: Value =
        serde_json::from_slice(bytes).map_err(|_| "Official update metadata is invalid JSON")?;
    if !metadata.is_array() {
        return Err("Official update metadata must be a release array".into());
    }
    Ok(metadata)
}

fn accepts_channel(version: &Version, channel: &str) -> bool {
    match channel {
        "stable" => version.pre.is_empty(),
        "preview" => !version.pre.is_empty(),
        _ => false,
    }
}
fn package_name(version: &Version) -> String {
    format!("Versora-{version}-win32-x64-setup.exe")
}
fn release_url(version: &Version) -> String {
    format!("{OFFICIAL_RELEASE_URL}/tag/v{version}")
}
fn package_url(version: &Version) -> String {
    format!(
        "{OFFICIAL_RELEASE_URL}/download/v{version}/{}",
        package_name(version)
    )
}

/// Discovery metadata only. Candidate identity is pinned and newer; publisher trust stays false.
pub fn select_candidate(
    releases: &Value,
    channel: &str,
    current: &str,
    highest: Option<&str>,
) -> Result<Option<Candidate>, String> {
    if !matches!(channel, "stable" | "preview") {
        return Err("Invalid update channel".into());
    }
    let current = strict_version(current)?;
    let floor = highest
        .map(strict_version)
        .transpose()?
        .map_or(current.clone(), |v| v.max(current));
    let rows = releases
        .as_array()
        .ok_or("Update release metadata must be an array")?;
    let mut selected: Option<(Version, Candidate)> = None;
    for row in rows {
        if row.get("draft").and_then(Value::as_bool) != Some(false) {
            continue;
        }
        let Some(tag) = row.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(raw_version) = tag.strip_prefix('v') else {
            continue;
        };
        let Ok(version) = strict_version(raw_version) else {
            continue;
        };
        if version <= floor || !accepts_channel(&version, channel) {
            continue;
        }
        if row.get("prerelease").and_then(Value::as_bool) != Some(!version.pre.is_empty()) {
            return Err("Release prerelease flag disagrees with its SemVer channel".into());
        }
        if row.get("html_url").and_then(Value::as_str) != Some(release_url(&version).as_str()) {
            return Err("Release URL does not match the pinned official Versora repository".into());
        }
        let assets = row
            .get("assets")
            .and_then(Value::as_array)
            .ok_or("Release assets are invalid")?;
        let name = package_name(&version);
        let matching: Vec<_> = assets
            .iter()
            .filter(|a| a.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        if matching.is_empty() {
            continue;
        } // e.g. historical browser/Python ZIP; not a native candidate.
        if matching.len() != 1 {
            return Err("Release has duplicate matching Windows NSIS assets".into());
        }
        let asset = matching[0];
        let size = asset
            .get("size")
            .and_then(Value::as_u64)
            .ok_or("Release asset size is invalid")?;
        if size == 0
            || size > PACKAGE_CAP
            || asset.get("state").and_then(Value::as_str) != Some("uploaded")
            || asset.get("browser_download_url").and_then(Value::as_str)
                != Some(package_url(&version).as_str())
        {
            return Err("Windows NSIS asset size, state or official URL is invalid".into());
        }
        let candidate = Candidate {
            version: version.to_string(),
            tag: tag.into(),
            asset_name: name,
            asset_size: size,
            release_url: release_url(&version),
            platform: "win32-x64".into(),
            format: "nsis".into(),
        };
        if selected.as_ref().is_none_or(|(old, _)| version > *old) {
            selected = Some((version, candidate));
        }
    }
    Ok(selected.map(|(_, candidate)| candidate))
}

fn patch_preferences(existing: &Preferences, patch: &Value) -> Result<Preferences, String> {
    let object = patch
        .as_object()
        .ok_or("Update preferences patch must be an object")?;
    let mut merged = serde_json::to_value(existing).expect("preferences serialize");
    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "policy" | "intervalHours" | "channel" | "autoDownload"
        ) {
            return Err(
                "Unknown update preference; URLs, paths and executable commands are not accepted"
                    .into(),
            );
        }
        merged[key] = value.clone();
    }
    let preferences: Preferences =
        serde_json::from_value(merged).map_err(|_| "Invalid update preference type")?;
    preferences.validate()?;
    Ok(preferences)
}

fn healthy_floor(saved: &Saved, current: &Version) -> Result<Version, String> {
    Ok(saved
        .highest_healthy_version
        .as_deref()
        .map(strict_version)
        .transpose()?
        .map_or(current.clone(), |v| v.max(current.clone())))
}
fn clear_obsolete_candidate(saved: &mut Saved, current: &Version) -> Result<(), String> {
    if let Some(candidate) = &saved.candidate {
        if candidate.validate(&saved.preferences.channel)? <= healthy_floor(saved, current)? {
            saved.candidate = None;
            saved.deferred = false;
        }
    }
    Ok(())
}
fn periodic_due(preferences: &Preferences, at: u64) -> Option<u64> {
    (preferences.policy == "periodic")
        .then(|| at.saturating_add(u64::from(preferences.interval_hours) * 3_600_000))
}
pub fn backoff_seconds(failures: u8) -> u64 {
    60u64
        .saturating_mul(1u64 << failures.saturating_sub(1).min(6))
        .min(3600)
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

async fn bounded_response(mut response: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > cap as u64)
    {
        return Err("Release metadata exceeds the 2 MiB bound".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cannot read official release metadata")?
    {
        if bytes.len().saturating_add(chunk.len()) > cap {
            return Err("Release metadata exceeds the 2 MiB bound".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn reject_links(path: &Path) -> Result<(), String> {
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(
                            "Update metadata paths cannot use symlinks or reparse points".into(),
                        );
                    }
                }
                if metadata.file_type().is_symlink() {
                    return Err("Update metadata paths cannot use symlinks".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect update metadata path".into()),
        }
    }
    Ok(())
}

fn read_saved(path: &Path) -> Result<Saved, String> {
    reject_links(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Saved::default()),
        Err(_) => return Err("Cannot read update state; original data was preserved".into()),
    };
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect update state")?
        .is_file()
    {
        return Err("Update state is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(STATE_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read update state")?;
    if bytes.len() as u64 > STATE_CAP {
        return Err("Update state exceeds its metadata bound; original preserved".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Update state is corrupt; original data was preserved".into())
}

fn lock_operation(path: &Path) -> Result<File, String> {
    reject_links(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.create(true).share_mode(0);
    }
    #[cfg(not(windows))]
    {
        options.create_new(true);
    } // On unsupported platforms, stale locks fail visibly; no guessed PID deletion.
    options.open(path).map_err(|_| {
        "Another process is updating metadata, or the update operation lock is unavailable".into()
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_links(path)?;
    let parent = path.parent().ok_or("Invalid update metadata destination")?;
    let temp = parent.join(format!(
        ".state-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::AcqRel)
    ));
    let write = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "Cannot reserve update metadata temporary file")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot persist update metadata; original preserved")?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err("Cannot atomically replace update metadata; original preserved".into());
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, path)
                .map_err(|_| "Cannot atomically replace update metadata; original preserved")?;
        }
        Ok(())
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}
