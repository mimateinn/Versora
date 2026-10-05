//! Suite contract v1 updater: GitHub release discovery, bounded signed package download and
//! verified installer handoff. A package is only ever executed after its minisign signature
//! (Tauri signer format) verifies against the public key compiled into this executable, and
//! it is verified again immediately before launch. Without a configured key, the updater stays
//! a metadata check with the official release page as the manual route.

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const OFFICIAL_RELEASE_URL: &str = "https://github.com/mimateinn/Versora/releases";
pub const CAPABILITY_REASON: &str = "Automatic download and installation are disabled: no reviewed publisher trust key is configured in this build. Use the official Versora release page.";
pub const INVALID_KEY_REASON: &str = "Automatic download and installation are disabled: the configured updater public key is invalid. Use the official Versora release page.";
pub const NOT_INSTALLED_REASON: &str = "Updates only work in the installed Versora app. This development or portable copy does not contact the update server.";
/// Committed trust anchor. Empty (or comments only) means no automatic download/install.
pub const PUBLIC_KEY_FILE: &str = include_str!("../updater-public-key.txt");
const METADATA_CAP: usize = 2 * 1024 * 1024;
const STATE_CAP: u64 = 256 * 1024;
const PACKAGE_CAP: u64 = 1024 * 1024 * 1024;
const SIGNATURE_CAP: usize = 16 * 1024;
const PUBLIC_KEY_CAP: usize = 4096;
pub const NOTES_CAP: usize = 8 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const PRODUCTION_MARKER: &[u8] = b"com.mimateinn.versora|Production";
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
    /// Suite default: check shortly after launch and every 4 hours, download automatically.
    fn default() -> Self {
        Self {
            policy: "periodic".into(),
            interval_hours: 4,
            channel: "stable".into(),
            auto_download: true,
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

/// Release notes are carried as bounded plain text. The renderer never receives HTML.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotes {
    pub version: String,
    pub text: String,
    pub truncated: bool,
}

/// Schema-1 state stays byte-compatible with earlier builds, so an older Versora can still
/// start on the same profile after a manual downgrade.
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

/// Updater facts that older builds do not know about live beside `state.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Extra {
    schema: u8,
    preferences_migrated: bool,
    last_seen_version: Option<String>,
    release_notes: Option<ReleaseNotes>,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            schema: 1,
            preferences_migrated: false,
            last_seen_version: None,
            release_notes: None,
        }
    }
}

/// How this executable may update itself.
#[derive(Clone, Debug, Default)]
pub struct UpdateConfig {
    /// Tauri signer public key: base64 of a minisign public key file.
    pub public_key: Option<String>,
    /// True only for the production per-user installation created by the NSIS installer.
    pub installed: bool,
    /// The first scheduled check waits this long after the service starts.
    pub startup_delay: Duration,
}

impl UpdateConfig {
    pub fn detect() -> Self {
        Self {
            public_key: configured_public_key(PUBLIC_KEY_FILE),
            installed: installed_copy(),
            startup_delay: Duration::from_secs(10),
        }
    }
}

struct Runtime {
    saved: Saved,
    extra: Extra,
    status: &'static str,
    error: Option<String>,
    /// Version whose package and signature on disk verified against the trust key.
    package: Option<String>,
    just_updated: Option<Value>,
}

pub struct UpdateService {
    directory: PathBuf,
    version: Version,
    trust: Option<PublicKey>,
    trust_error: Option<&'static str>,
    installed: bool,
    startup_due: Instant,
    runtime: Mutex<Runtime>,
    active: AtomicBool,
    cancellation: AtomicU64,
    started: AtomicBool,
    received: AtomicU64,
    total: AtomicU64,
}

/// A verified package whose file handle stays open (on Windows without write/delete sharing)
/// until the installer process has been created from it.
pub struct VerifiedPackage {
    pub path: PathBuf,
    pub version: String,
    _file: File,
}

/// Source of package bytes; production reads the HTTPS response, tests use fixtures.
pub trait ChunkSource {
    fn next_chunk(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, String>> + Send;
}

struct ResponseSource(reqwest::Response);
impl ChunkSource for ResponseSource {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.0
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|_| "The update download was interrupted".into())
    }
}

#[cfg(test)]
pub struct FixtureSource {
    pub chunks: std::collections::VecDeque<Vec<u8>>,
    pub delay: Duration,
}
#[cfg(test)]
impl ChunkSource for FixtureSource {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        tokio::time::sleep(self.delay).await;
        Ok(self.chunks.pop_front())
    }
}

impl UpdateService {
    pub fn new(data_dir: PathBuf, version: &str) -> Result<Self, String> {
        Self::with_config(data_dir, version, UpdateConfig::detect())
    }

    pub fn with_config(
        data_dir: PathBuf,
        version: &str,
        config: UpdateConfig,
    ) -> Result<Self, String> {
        let version = strict_version(version)?;
        if !data_dir.is_absolute() {
            return Err("Update user-data directory must be absolute".into());
        }
        reject_links(&data_dir)?;
        let directory = data_dir.join("updates");
        reject_links(&directory)?;
        let state_path = directory.join("state.json");
        let existed = state_path.exists();
        let mut saved = read_saved(&state_path)?;
        saved.validate()?;
        // A restarted interrupted metadata check is cancelled, never 'ready'.
        let interrupted = saved.in_progress;
        saved.in_progress = false;
        clear_obsolete_candidate(&mut saved, &version)?;
        fs::create_dir_all(&directory).map_err(|_| "Cannot create update metadata directory")?;
        let extra = read_extra(&directory.join("updater.json"));
        let (trust, trust_error) = match config.public_key.as_deref().map(decode_public_key) {
            None => (None, None),
            Some(Ok(key)) => (Some(key), None),
            Some(Err(_)) => (None, Some(INVALID_KEY_REASON)),
        };
        let service = Self {
            directory,
            version,
            trust,
            trust_error,
            installed: config.installed,
            startup_due: Instant::now() + config.startup_delay,
            runtime: Mutex::new(Runtime {
                status: "idle",
                saved,
                extra,
                error: interrupted.then(|| {
                    "Previous metadata check was interrupted; no package was downloaded".into()
                }),
                package: None,
                just_updated: None,
            }),
            active: AtomicBool::new(false),
            cancellation: AtomicU64::new(0),
            started: AtomicBool::new(false),
            received: AtomicU64::new(0),
            total: AtomicU64::new(0),
        };
        service.migrate_preferences(existed);
        {
            let mut state = service.lock();
            if service.trust.is_some() && service.verify_ready_package_in(&state).is_ok() {
                state.package = state.saved.candidate.as_ref().map(|c| c.version.clone());
            }
            state.status = if interrupted {
                "cancelled"
            } else {
                status_for(&state)
            };
        }
        Ok(service)
    }

    /// Profiles from builds before the automatic updater kept the old defaults
    /// (startup/24 h, download unavailable). Those defaults move once to the suite defaults;
    /// explicit manual or periodic choices and the channel are kept.
    fn migrate_preferences(&self, existed: bool) {
        let mut state = self.lock();
        if state.extra.preferences_migrated || !existed {
            return;
        }
        let Ok(operation) = self.begin_locked() else {
            return; // another process holds the lock; try again on a later launch
        };
        let mut saved = state.saved.clone();
        if saved.preferences.policy == "startup" && saved.preferences.interval_hours == 24 {
            saved.preferences.policy = "periodic".into();
            saved.preferences.interval_hours = 4;
        }
        // The old build never enabled the download toggle, so `false` was not a user choice.
        saved.preferences.auto_download = true;
        let mut extra = state.extra.clone();
        extra.preferences_migrated = true;
        if self.save(&saved).is_ok() && self.save_extra(&extra).is_ok() {
            state.saved = saved;
            state.extra = extra;
        }
        drop(operation);
    }

    pub fn official_release_url(&self) -> &'static str {
        OFFICIAL_RELEASE_URL
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn trusted(&self) -> bool {
        self.trust.is_some()
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Why automatic download/install is unavailable, if it is.
    pub fn capability_reason(&self) -> Option<&'static str> {
        if !self.installed {
            Some(NOT_INSTALLED_REASON)
        } else if let Some(reason) = self.trust_error {
            Some(reason)
        } else if self.trust.is_none() {
            Some(CAPABILITY_REASON)
        } else {
            None
        }
    }

    pub fn require_capability(&self) -> Result<(), String> {
        match self.capability_reason() {
            Some(reason) => Err(reason.into()),
            None => Ok(()),
        }
    }

    pub fn get_preferences(&self) -> Value {
        serde_json::to_value(&self.lock().saved.preferences).expect("preferences serialize")
    }

    pub fn get_state(&self) -> Value {
        let state = self.lock();
        let saved = &state.saved;
        let capable = self.capability_reason().is_none();
        let ready = state.status == "ready";
        json!({ "status": state.status, "currentVersion": self.version.to_string(),
            "candidateVersion": saved.candidate.as_ref().map(|c| c.version.as_str()), "candidate": saved.candidate,
            "preferences": saved.preferences, "channel": saved.preferences.channel,
            "bytesReceived": self.received.load(Ordering::Acquire), "totalBytes": self.total.load(Ordering::Acquire),
            "error": state.error,
            "lastCheckAt": saved.last_check_at, "nextCheckAt": saved.next_check_at,
            "deferred": saved.deferred, "active": self.active.load(Ordering::Acquire),
            "highestHealthyVersion": saved.highest_healthy_version,
            "trust": self.trust.is_some(), "installed": self.installed,
            "installCapabilities": capable, "downloadCapability": capable,
            "capabilityReason": self.capability_reason(), "officialReleaseUrl": OFFICIAL_RELEASE_URL,
            "recoveryCapability": false, "packageDownloaded": ready,
            "installOnQuit": ready && saved.preferences.auto_download,
            "releaseNotes": state.extra.release_notes, "justUpdated": state.just_updated })
    }

    pub fn set_preferences(&self, patch: Value) -> Result<Value, String> {
        // Validate before signalling cancellation so bad renderer input has no side effects.
        patch_preferences(&self.lock().saved.preferences, &patch)?;
        if self.active.load(Ordering::Acquire) {
            self.cancellation.fetch_add(1, Ordering::AcqRel);
            return Err(
                "Update check is stopping; save these preferences again after it stops".into(),
            );
        }
        let operation = self.begin()?;
        let mut state = self.lock();
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
        if changed_channel {
            state.package = None;
        }
        if !state.extra.preferences_migrated {
            let mut extra = state.extra.clone();
            extra.preferences_migrated = true;
            if self.save_extra(&extra).is_ok() {
                state.extra = extra;
            }
        }
        state.status = status_for(&state);
        state.error = None;
        if changed_policy || changed_channel {
            self.started.store(false, Ordering::Release);
        }
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    /// Manual checks bypass the schedule/backoff. One update operation per data directory.
    pub async fn check(&self) -> Result<Value, String> {
        if self.active.load(Ordering::Acquire) {
            return Ok(self.get_state());
        }
        if !self.installed {
            return Err(NOT_INSTALLED_REASON.into());
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

    /// Downloads and verifies the current candidate. `progress` is called (throttled) while
    /// bytes arrive so the shell can publish state to the renderer.
    pub async fn download(&self, progress: &(dyn Fn() + Send + Sync)) -> Result<Value, String> {
        if self.active.load(Ordering::Acquire) {
            return Ok(self.get_state());
        }
        self.require_capability()?;
        let operation = self.begin()?;
        let Some((candidate, signature_url, package_url, generation)) = self.start_download()?
        else {
            drop(operation);
            return Ok(self.get_state());
        };
        let result = tokio::select! {
            biased;
            _ = self.cancelled(generation) => Err("Update download cancelled".to_owned()),
            result = async {
                let client = download_client(&self.version)?;
                let signature = fetch_signature(&client, &signature_url).await?;
                let response = client
                    .get(&package_url)
                    .send()
                    .await
                    .map_err(|_| "The update package request failed".to_owned())?;
                if !response.status().is_success() {
                    return Err(format!("The update package returned HTTP {}", response.status().as_u16()));
                }
                if response.content_length().is_some_and(|length| length != candidate.asset_size) {
                    return Err("The update package size differs from its release metadata".into());
                }
                self.receive_package(&candidate, &signature, ResponseSource(response), generation, progress).await
            } => result,
        };
        let result = self.finish_download(&candidate, result, generation);
        drop(operation);
        result.map(|_| self.get_state())
    }

    #[cfg(test)]
    pub async fn download_fixture(
        &self,
        source: FixtureSource,
        signature: Option<&str>,
    ) -> Result<Value, String> {
        self.require_capability()?;
        let operation = self.begin()?;
        let Some((candidate, _, _, generation)) = self.start_download()? else {
            drop(operation);
            return Ok(self.get_state());
        };
        let result = tokio::select! {
            biased;
            _ = self.cancelled(generation) => Err("Update download cancelled".to_owned()),
            result = async {
                let signature = signature.ok_or("This release has no updater signature; use the official release page")?;
                self.receive_package(&candidate, signature, source, generation, &|| {}).await
            } => result,
        };
        let result = self.finish_download(&candidate, result, generation);
        drop(operation);
        result.map(|_| self.get_state())
    }

    /// True when the shell should start a background download now.
    pub fn auto_download_due(&self) -> bool {
        if self.capability_reason().is_some() || self.active.load(Ordering::Acquire) {
            return false;
        }
        let state = self.lock();
        state.saved.preferences.auto_download && state.status == "available"
    }

    /// True when a verified package should be installed silently as the app quits.
    pub fn install_on_quit_ready(&self) -> bool {
        if self.capability_reason().is_some() || self.active.load(Ordering::Acquire) {
            return false;
        }
        let state = self.lock();
        state.saved.preferences.auto_download && state.status == "ready"
    }

    pub fn cancel(&self) -> Result<Value, String> {
        self.cancellation.fetch_add(1, Ordering::AcqRel);
        if self.active.load(Ordering::Acquire) {
            self.lock().error = Some("Stopping the update operation".into());
            return Ok(self.get_state()); // active remains true until the request future has dropped.
        }
        let _operation = self.begin()?;
        let mut state = self.lock();
        self.refresh(&mut state)?;
        state.status = if state.status == "ready" {
            "ready"
        } else {
            "cancelled"
        };
        state.error = None;
        drop(state);
        drop(_operation);
        Ok(self.get_state())
    }

    pub fn later(&self) -> Result<Value, String> {
        let operation = self.begin()?;
        let mut state = self.lock();
        self.refresh(&mut state)?;
        if state.saved.candidate.is_none() {
            return Err("No available update metadata to defer".into());
        }
        let mut saved = state.saved.clone();
        saved.deferred = true;
        self.save(&saved)?;
        state.saved = saved;
        state.status = status_for(&state);
        state.error = None;
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    /// Caller combines native job/persistence truth with renderer unsaved-edit truth. Active
    /// or unsaved work needs the user's explicit confirmation (`confirmed`); pending
    /// persistence always refuses. Approval re-verifies the package but does not launch it:
    /// the shell first runs its safe-exit path and then calls `launch_installer`.
    pub fn request_install(&self, work_snapshot: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Work {
            busy: bool,
            unsaved: bool,
            pending_persistence: bool,
            #[serde(default)]
            confirmed: bool,
        }
        let work: Work =
            serde_json::from_value(work_snapshot).map_err(|_| "Invalid update work snapshot")?;
        if work.busy && !work.confirmed {
            return Err("Finish or cancel active translation jobs before updating".into());
        }
        if work.unsaved && !work.confirmed {
            return Err("Save or discard unsaved work before updating".into());
        }
        if work.pending_persistence {
            return Err(
                "Wait for settings and project data to finish saving before updating".into(),
            );
        }
        let operation = self.begin()?;
        let mut state = self.lock();
        self.refresh(&mut state)?;
        if let Some(reason) = self.capability_reason() {
            state.error = Some(reason.into());
            drop(state);
            drop(operation);
            let mut result = self.get_state();
            result["installBlocked"] = json!(true);
            result["appRemainsOpen"] = json!(true);
            return Ok(result);
        }
        let verified = self
            .verify_ready_package_in(&state)
            .map(|package| package.version);
        match &verified {
            Ok(version) => state.package = Some(version.clone()),
            Err(error) => {
                state.package = None;
                state.error = Some(error.clone());
            }
        }
        state.status = status_for(&state);
        drop(state);
        drop(operation);
        verified?;
        let mut result = self.get_state();
        result["installApproved"] = json!(true);
        result["appRemainsOpen"] = json!(false);
        Ok(result)
    }

    /// Verifies the ready package again and starts its installer detached:
    /// `/S /UPDATE` (+ `/RELAUNCH`). The caller exits the app right after success.
    pub fn launch_installer(&self, relaunch: bool) -> Result<(), String> {
        self.require_capability()?;
        let operation = self.begin()?;
        let mut state = self.lock();
        self.refresh(&mut state)?;
        let package = match self.verify_ready_package_in(&state) {
            Ok(package) => package,
            Err(error) => {
                state.package = None;
                state.status = status_for(&state);
                state.error = Some(error.clone());
                return Err(error);
            }
        };
        let launched = spawn_installer(&package, relaunch);
        if let Err(error) = &launched {
            state.error = Some(error.clone());
        }
        drop(package);
        drop(state);
        drop(operation);
        launched
    }

    /// Re-reads and verifies the candidate package and signature from disk.
    pub fn verify_ready_package(&self) -> Result<VerifiedPackage, String> {
        let state = self.lock();
        self.verify_ready_package_in(&state)
    }

    fn verify_ready_package_in(&self, state: &Runtime) -> Result<VerifiedPackage, String> {
        let key = self.trust.as_ref().ok_or(CAPABILITY_REASON)?;
        let candidate = state
            .saved
            .candidate
            .as_ref()
            .ok_or("No downloaded update is ready to install")?;
        let version = candidate.validate(&state.saved.preferences.channel)?;
        let path = self.directory.join(&candidate.asset_name);
        let signature_path = self.directory.join(format!("{}.sig", candidate.asset_name));
        reject_links(&path)?;
        reject_links(&signature_path)?;
        if !path.is_file() {
            return Err("No downloaded update is ready to install".into());
        }
        let signature = read_text(&signature_path, SIGNATURE_CAP)
            .map_err(|_| "The downloaded update has no readable signature".to_owned())?;
        let file = open_package_for_launch(&path)?;
        let length = file
            .metadata()
            .map_err(|_| "Cannot inspect the downloaded update")?
            .len();
        if length != candidate.asset_size {
            return Err("The downloaded update size differs from its release metadata".into());
        }
        verify_reader(key, &file, &signature, &version)?;
        Ok(VerifiedPackage {
            path,
            version: version.to_string(),
            _file: file,
        })
    }

    /// Explicit native/UI-ready ACK only. A version marker is not an installation/rollback claim.
    pub fn health_ack(&self) -> Result<Value, String> {
        let operation = self.begin()?;
        let mut state = self.lock();
        self.refresh(&mut state)?;
        let mut saved = state.saved.clone();
        let mut extra = state.extra.clone();
        let previous = extra
            .last_seen_version
            .as_deref()
            .or(saved.highest_healthy_version.as_deref())
            .and_then(|v| strict_version(v).ok());
        let highest = healthy_floor(&saved, &self.version)?;
        saved.highest_healthy_version = Some(highest.to_string());
        clear_obsolete_candidate(&mut saved, &self.version)?;
        self.save(&saved)?;
        state.saved = saved;
        if let Some(previous) = previous.filter(|previous| *previous < self.version) {
            let notes = extra
                .release_notes
                .clone()
                .filter(|notes| notes.version == self.version.to_string());
            state.just_updated = Some(json!({ "from": previous.to_string(),
                "to": self.version.to_string(), "releaseNotes": notes }));
        }
        extra.last_seen_version = Some(self.version.to_string());
        extra.preferences_migrated = true;
        if extra
            .release_notes
            .as_ref()
            .is_some_and(|notes| strict_version(&notes.version).map_or(true, |v| v < self.version))
        {
            extra.release_notes = None;
        }
        if self.save_extra(&extra).is_ok() {
            state.extra = extra;
        }
        if state
            .package
            .as_ref()
            .is_some_and(|v| state.saved.candidate.as_ref().map(|c| &c.version) != Some(v))
        {
            state.package = None;
        }
        let keep = state.package.clone();
        self.prune_packages(keep.as_deref());
        if matches!(state.status, "available" | "ready" | "idle") {
            state.status = status_for(&state);
        }
        drop(state);
        drop(operation);
        Ok(self.get_state())
    }

    pub fn seconds_until_next_check(&self) -> Option<u64> {
        if self.active.load(Ordering::Acquire) || !self.installed {
            return None;
        }
        let state = self.lock();
        if state.saved.preferences.policy == "manual" {
            return None;
        }
        if !self.started.load(Ordering::Acquire) {
            let wait = self.startup_due.saturating_duration_since(Instant::now());
            return Some(wait.as_millis().div_ceil(1000) as u64);
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

    /// Takes the operation lock while the caller already holds the runtime mutex. The
    /// returned guard must be dropped after the mutex guard is released by the caller's scope
    /// ordering, so it never touches the runtime itself.
    fn begin_locked(&self) -> Result<RawOperation<'_>, String> {
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("An update operation is already active".into());
        }
        match lock_operation(&self.directory.join("operation.lock")) {
            Ok(file) => Ok(RawOperation {
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
        if state.package.is_some()
            && state.saved.candidate.as_ref().map(|c| &c.version) != state.package.as_ref()
        {
            state.package = None;
        }
        Ok(())
    }

    fn start_check(&self) -> Result<u64, String> {
        let mut state = self.lock();
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
        let mut state = self.lock();
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
        if state.package.is_some()
            && state.saved.candidate.as_ref().map(|c| &c.version) != state.package.as_ref()
        {
            state.package = None;
        }
        let notes = state
            .saved
            .candidate
            .as_ref()
            .and_then(|candidate| release_notes(&releases, &candidate.version));
        let keep_current = state
            .extra
            .release_notes
            .as_ref()
            .is_some_and(|notes| notes.version == self.version.to_string());
        if notes.is_some() || !keep_current {
            let mut extra = state.extra.clone();
            extra.release_notes = notes;
            if self.save_extra(&extra).is_ok() {
                state.extra = extra;
            }
        }
        state.status = status_for(&state);
        state.error = None;
        Ok(())
    }

    fn finish_error(&self, error: String, at: u64, generation: u64) -> Result<(), String> {
        let cancelled = self.cancellation.load(Ordering::Acquire) != generation;
        let mut state = self.lock();
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

    /// Returns `None` when the candidate's package is already verified on disk.
    fn start_download(&self) -> Result<Option<(Candidate, String, String, u64)>, String> {
        let mut state = self.lock();
        self.refresh(&mut state)?;
        let candidate = state
            .saved
            .candidate
            .clone()
            .ok_or("No update is available to download")?;
        let version = candidate.validate(&state.saved.preferences.channel)?;
        if state.package.as_deref() == Some(candidate.version.as_str())
            && self.verify_ready_package_in(&state).is_ok()
        {
            state.status = "ready";
            return Ok(None);
        }
        state.package = None;
        state.status = "downloading";
        state.error = None;
        self.received.store(0, Ordering::Release);
        self.total.store(candidate.asset_size, Ordering::Release);
        let package = package_url(&version);
        Ok(Some((
            candidate,
            format!("{package}.sig"),
            package,
            self.cancellation.load(Ordering::Acquire),
        )))
    }

    /// Streams the package into an owned temporary file, verifies size and signature, then
    /// atomically moves it (and its signature) into place.
    async fn receive_package(
        &self,
        candidate: &Candidate,
        signature: &str,
        mut source: impl ChunkSource + Send,
        generation: u64,
        progress: &(dyn Fn() + Send + Sync),
    ) -> Result<(), String> {
        let key = self.trust.as_ref().ok_or(CAPABILITY_REASON)?;
        let version = strict_version(&candidate.version)?;
        let parsed = parse_signature(signature)?;
        let mut verifier = key
            .verify_stream(&parsed)
            .map_err(|_| "The update signature was not made with this app's publisher key")?;
        let temp = TempFile::create(&self.directory)?;
        let mut file = temp.open()?;
        let mut received = 0u64;
        let mut reported = Instant::now();
        loop {
            if self.cancellation.load(Ordering::Acquire) != generation {
                return Err("Update download cancelled".into());
            }
            let chunk = tokio::time::timeout(CHUNK_TIMEOUT, source.next_chunk())
                .await
                .map_err(|_| "The update download stalled for 60 seconds".to_owned())??;
            let Some(chunk) = chunk else { break };
            received = received.saturating_add(chunk.len() as u64);
            if received > candidate.asset_size || received > PACKAGE_CAP {
                return Err("The update package is larger than its release metadata".into());
            }
            file.write_all(&chunk)
                .map_err(|_| "Cannot write the update package; check free disk space")?;
            verifier.update(&chunk);
            self.received.store(received, Ordering::Release);
            if reported.elapsed() >= PROGRESS_INTERVAL {
                reported = Instant::now();
                progress();
            }
        }
        if received != candidate.asset_size {
            return Err("The update download ended early".into());
        }
        file.sync_all()
            .map_err(|_| "Cannot persist the update package")?;
        drop(file);
        verifier
            .finalize()
            .map_err(|_| "The update package signature is invalid; it was discarded")?;
        check_signed_identity(&parsed, &version)?;
        let destination = self.directory.join(&candidate.asset_name);
        atomic_write(
            &self.directory.join(format!("{}.sig", candidate.asset_name)),
            signature.trim().as_bytes(),
        )?;
        temp.commit(&destination)?;
        progress();
        Ok(())
    }

    fn finish_download(
        &self,
        candidate: &Candidate,
        result: Result<(), String>,
        generation: u64,
    ) -> Result<(), String> {
        let cancelled = self.cancellation.load(Ordering::Acquire) != generation;
        let mut state = self.lock();
        match result {
            Ok(()) => {
                state.package = Some(candidate.version.clone());
                state.error = None;
                state.status = status_for(&state);
                self.prune_packages(Some(&candidate.version));
                Ok(())
            }
            Err(error) => {
                state.package = None;
                state.status = if cancelled { "cancelled" } else { "failed" };
                state.error = Some(error.clone());
                if cancelled {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Removes downloaded packages, signatures and partial files other than `keep`.
    /// Only exact owned names inside the updates directory are touched.
    fn prune_packages(&self, keep: Option<&str>) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let owned = if let Some(rest) = name.strip_prefix("Versora-") {
                let stem = rest
                    .strip_suffix("-win32-x64-setup.exe.sig")
                    .or_else(|| rest.strip_suffix("-win32-x64-setup.exe"));
                stem.is_some_and(|v| strict_version(v).is_ok() && Some(v) != keep)
            } else {
                name.starts_with(".download-") && name.ends_with(".part")
            };
            if owned
                && entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn save(&self, saved: &Saved) -> Result<(), String> {
        saved.validate()?;
        let bytes =
            serde_json::to_vec_pretty(saved).map_err(|_| "Cannot serialize update state")?;
        atomic_write(&self.directory.join("state.json"), &bytes)
    }

    fn save_extra(&self, extra: &Extra) -> Result<(), String> {
        let bytes =
            serde_json::to_vec_pretty(extra).map_err(|_| "Cannot serialize update state")?;
        atomic_write(&self.directory.join("updater.json"), &bytes)
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

fn status_for(state: &Runtime) -> &'static str {
    match &state.saved.candidate {
        None => "idle",
        Some(candidate) if state.package.as_ref() == Some(&candidate.version) => "ready",
        Some(_) => "available",
    }
}

struct Operation<'a> {
    service: &'a UpdateService,
    file: Option<File>,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut state = self.service.lock();
        if state.status == "checking" {
            state.saved.in_progress = false;
            state.status = "cancelled";
            state.error = Some("Metadata operation interrupted; no package was downloaded".into());
            if let Err(error) = self.service.save(&state.saved) {
                state.status = "failed";
                state.error = Some(error);
            }
        } else if state.status == "downloading" {
            state.status = "cancelled";
            state.error = Some("The update download was interrupted".into());
        }
        drop(state);
        release_operation(self.service, self.file.take());
    }
}

/// Operation guard used while the runtime mutex is already held by the caller.
struct RawOperation<'a> {
    service: &'a UpdateService,
    file: Option<File>,
}
impl Drop for RawOperation<'_> {
    fn drop(&mut self) {
        release_operation(self.service, self.file.take());
    }
}

fn release_operation(service: &UpdateService, file: Option<File>) {
    drop(file);
    #[cfg(not(windows))]
    {
        let _ = fs::remove_file(service.directory.join("operation.lock"));
    }
    service.active.store(false, Ordering::Release);
}

/// Owned partial download; removed on drop unless committed.
struct TempFile {
    path: PathBuf,
    committed: bool,
}
impl TempFile {
    fn create(directory: &Path) -> Result<Self, String> {
        let path = directory.join(format!(
            ".download-{}-{}.part",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::AcqRel)
        ));
        reject_links(&path)?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "Cannot reserve the update download file")?;
        Ok(Self {
            path,
            committed: false,
        })
    }
    fn open(&self) -> Result<File, String> {
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|_| "Cannot open the update download file".into())
    }
    fn commit(mut self, destination: &Path) -> Result<(), String> {
        atomic_replace(&self.path, destination)?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
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
        // Preview also follows stable releases, so it is never stuck on an
        // older prerelease once the stable version ships.
        "preview" => true,
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

/// Discovery metadata only. Candidate identity is pinned and newer; publisher trust comes
/// from the package signature, never from this metadata.
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

/// The release `body` of `version`, reduced to bounded plain text.
pub fn release_notes(releases: &Value, version: &str) -> Option<ReleaseNotes> {
    let tag = format!("v{version}");
    let row = releases
        .as_array()?
        .iter()
        .find(|row| row.get("tag_name").and_then(Value::as_str) == Some(tag.as_str()))?;
    Some(bound_release_notes(
        version,
        row.get("body").and_then(Value::as_str).unwrap_or(""),
    ))
}

pub fn bound_release_notes(version: &str, body: &str) -> ReleaseNotes {
    let mut text = String::new();
    let mut truncated = false;
    let mut newlines = 0;
    for ch in body.replace("\r\n", "\n").chars() {
        let ch = if ch == '\r' { '\n' } else { ch };
        // Control and bidirectional-override characters never reach the renderer.
        if (ch.is_control() && ch != '\n' && ch != '\t')
            || matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
        {
            continue;
        }
        newlines = if ch == '\n' { newlines + 1 } else { 0 };
        if newlines > 2 {
            continue; // Runs of blank lines collapse to at most one blank line.
        }
        if text.len() + ch.len_utf8() > NOTES_CAP {
            truncated = true;
            break;
        }
        text.push(ch);
    }
    ReleaseNotes {
        version: version.into(),
        text: text.trim().into(),
        truncated,
    }
}

/// Trust anchor text: `#` comment lines and blank lines are ignored.
pub fn configured_public_key(text: &str) -> Option<String> {
    let key: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    (!key.is_empty()).then_some(key)
}

fn decode_base64_text(value: &str, cap: usize, what: &str) -> Result<String, String> {
    let compact: String = value.split_ascii_whitespace().collect();
    if compact.is_empty() || compact.len() > cap {
        return Err(format!("The {what} is empty or too large"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| format!("The {what} is not valid base64"))?;
    String::from_utf8(bytes).map_err(|_| format!("The {what} is not UTF-8 text"))
}

/// Tauri signer public key: base64 of the minisign public key file.
pub fn decode_public_key(encoded: &str) -> Result<PublicKey, String> {
    let text = decode_base64_text(encoded, PUBLIC_KEY_CAP, "updater public key")?;
    PublicKey::decode(&text).map_err(|_| "The updater public key is not a minisign key".into())
}

/// Tauri signer signature: base64 of the minisign signature file.
fn parse_signature(encoded: &str) -> Result<Signature, String> {
    let text = decode_base64_text(encoded, SIGNATURE_CAP, "update signature")?;
    Signature::decode(&text).map_err(|_| "The update signature is malformed".into())
}

/// Only call after the global signature verified: the trusted comment is then authentic.
/// It must name exactly this version (`signer sign --app-version`), and when it names a file,
/// exactly this package.
fn check_signed_identity(signature: &Signature, version: &Version) -> Result<(), String> {
    let mut signed_version = None;
    let mut signed_file = None;
    for field in signature.trusted_comment().split('\t') {
        if let Some(value) = field.strip_prefix("version:") {
            signed_version = Some(value.trim());
        } else if let Some(value) = field.strip_prefix("file:") {
            signed_file = Some(value.trim());
        }
    }
    match signed_version.map(|v| strict_version(v.trim_start_matches('v'))) {
        Some(Ok(signed)) if signed == *version => {}
        Some(_) => return Err("The update signature was made for a different version".into()),
        None => return Err("The update signature does not name the version it signs".into()),
    }
    if signed_file.is_some_and(|file| file != package_name(version)) {
        return Err("The update signature was made for a different file".into());
    }
    Ok(())
}

/// Streams `reader` through the signature check (prehashed Ed25519/BLAKE2b, no legacy mode).
pub fn verify_reader(
    key: &PublicKey,
    mut reader: impl Read,
    signature: &str,
    version: &Version,
) -> Result<u64, String> {
    let parsed = parse_signature(signature)?;
    let mut verifier = key
        .verify_stream(&parsed)
        .map_err(|_| "The update signature was not made with this app's publisher key")?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| "Cannot read the downloaded update")?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > PACKAGE_CAP {
            return Err("The update package exceeds the size bound".into());
        }
        verifier.update(&buffer[..read]);
    }
    verifier
        .finalize()
        .map_err(|_| "The update package signature is invalid".to_owned())?;
    check_signed_identity(&parsed, version)?;
    Ok(total)
}

pub fn verify_package_bytes(
    public_key: &str,
    data: &[u8],
    signature: &str,
    version: &str,
) -> Result<(), String> {
    verify_reader(
        &decode_public_key(public_key)?,
        data,
        signature,
        &strict_version(version)?,
    )
    .map(|_| ())
}

fn installed_copy() -> bool {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let Some(directory) = exe.parent() else {
            return false;
        };
        exe.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("versora.exe"))
            && read_text(&directory.join(".versora-installation"), 256)
                .is_ok_and(|marker| marker.trim().as_bytes() == PRODUCTION_MARKER)
    }
    #[cfg(not(windows))]
    {
        let _ = PRODUCTION_MARKER;
        false
    }
}

/// Opens the package for verification and keeps it open until the installer has started.
/// On Windows the handle denies write and delete sharing, so the verified bytes are the
/// bytes that are executed.
pub fn open_package_for_launch(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x0000_0001); // FILE_SHARE_READ
    }
    options
        .open(path)
        .map_err(|_| "Cannot open the downloaded update".into())
}

fn spawn_installer(package: &VerifiedPackage, relaunch: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let mut command = std::process::Command::new(&package.path);
        command.args(["/S", "/UPDATE"]);
        if relaunch {
            command.arg("/RELAUNCH");
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map(|_| ())
            .map_err(|_| "Cannot start the verified Versora installer".into())
    }
    #[cfg(not(windows))]
    {
        let _ = (package, relaunch);
        Err("Installer handoff is only available on Windows".into())
    }
}

fn download_client(version: &Version) -> Result<reqwest::Client, String> {
    // GitHub answers release downloads with a redirect to its asset CDN. Only HTTPS hops to
    // GitHub-owned hosts are followed; the signature authenticates the bytes regardless.
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        let allowed = attempt.url().scheme() == "https"
            && attempt.url().host_str().is_some_and(|host| {
                host == "github.com"
                    || host == "objects.githubusercontent.com"
                    || host == "release-assets.githubusercontent.com"
            });
        if attempt.previous().len() >= 5 || !allowed {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(redirects)
        .connect_timeout(Duration::from_secs(15))
        .user_agent(format!("Versora/{version}"))
        .build()
        .map_err(|_| "Cannot create HTTPS update download client".into())
}

async fn fetch_signature(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = tokio::time::timeout(TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| "The update signature request timed out".to_owned())?
        .map_err(|_| "The update signature request failed".to_owned())?;
    if response.status().as_u16() == 404 {
        return Err("This release has no updater signature; use the official release page".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "The update signature returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = tokio::time::timeout(TIMEOUT, bounded_response(response, SIGNATURE_CAP))
        .await
        .map_err(|_| "The update signature request timed out".to_owned())?
        .map_err(|_| "The update signature is too large".to_owned())?;
    String::from_utf8(bytes).map_err(|_| "The update signature is not text".into())
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

fn read_bounded(path: &Path, cap: u64) -> Result<Option<Vec<u8>>, String> {
    reject_links(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read update state")?;
    if bytes.len() as u64 > cap {
        return Err("Update state exceeds its metadata bound; original preserved".into());
    }
    Ok(Some(bytes))
}

fn read_text(path: &Path, cap: usize) -> Result<String, String> {
    let bytes = read_bounded(path, cap as u64)?.ok_or("File is missing")?;
    String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text".into())
}

fn read_saved(path: &Path) -> Result<Saved, String> {
    match read_bounded(path, STATE_CAP)? {
        None => Ok(Saved::default()),
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Update state is corrupt; original data was preserved".into()),
    }
}

/// Non-critical updater facts: a missing or unreadable file starts fresh.
fn read_extra(path: &Path) -> Extra {
    read_bounded(path, STATE_CAP)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<Extra>(&bytes).ok())
        .filter(|extra| extra.schema == 1)
        .map(|mut extra| {
            if extra
                .last_seen_version
                .as_deref()
                .is_some_and(|v| strict_version(v).is_err())
            {
                extra.last_seen_version = None;
            }
            if let Some(notes) = &extra.release_notes {
                if strict_version(&notes.version).is_err() || notes.text.len() > NOTES_CAP {
                    extra.release_notes = None;
                }
            }
            extra
        })
        .unwrap_or_default()
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

fn atomic_replace(from: &Path, to: &Path) -> Result<(), String> {
    reject_links(to)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let source: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err("Cannot atomically replace update metadata; original preserved".into());
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)
            .map_err(|_| "Cannot atomically replace update metadata; original preserved")?;
    }
    Ok(())
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
        atomic_replace(&temp, path)
    })();
    if write.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write
}
