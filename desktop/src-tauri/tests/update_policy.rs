#[path = "../src/suite_updates.rs"]
mod suite_updates;

use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use suite_updates::{FixtureSource, UpdateConfig, UpdateService, OFFICIAL_RELEASE_URL};
static NEXT: AtomicU64 = AtomicU64::new(0);

// TEST-ONLY signing fixtures. Both keys are throwaway keys made with
// `npx @tauri-apps/cli@2.12.1 signer generate` for these tests; their private halves were
// never committed and they are not Versora's publisher key. The package is 32 KiB of random
// bytes signed as `Versora-9.9.9-win32-x64-setup.exe` with `signer sign --app-version 9.9.9`
// (plus variants without a version, for 9.9.8 and with the other key).
const TRUSTED_KEY: &str = include_str!("fixtures/updater/test-only-trusted.pub");
const OTHER_KEY: &str = include_str!("fixtures/updater/test-only-other.pub");
const PACKAGE: &[u8] = include_bytes!("fixtures/updater/package-9.9.9.bin");
const SIGNATURE: &str = include_str!("fixtures/updater/package-9.9.9.sig");
const SIGNATURE_UNVERSIONED: &str = include_str!("fixtures/updater/package-9.9.9-unversioned.sig");
const SIGNATURE_FOR_998: &str = include_str!("fixtures/updater/package-9.9.9-signed-as-9.9.8.sig");
const SIGNATURE_OTHER_KEY: &str = include_str!("fixtures/updater/package-9.9.9-other-key.sig");

struct Temp {
    root: PathBuf,
    path: PathBuf,
}
impl Temp {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/update-test-data");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self { root, path }
    }
    /// The installed app without a trust key (today's shipped configuration).
    fn service(&self, version: &str) -> UpdateService {
        self.configured(version, None, true)
    }
    /// The installed app with the test-only trust key.
    fn trusted(&self, version: &str) -> UpdateService {
        self.configured(version, Some(TRUSTED_KEY), true)
    }
    fn configured(&self, version: &str, key: Option<&str>, installed: bool) -> UpdateService {
        UpdateService::with_config(
            self.path.clone(),
            version,
            UpdateConfig {
                public_key: key.map(Into::into),
                installed,
            },
        )
        .unwrap()
    }
    fn updates(&self) -> PathBuf {
        self.path.join("updates")
    }
    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(self.updates())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.ends_with(".part") || name.ends_with(".exe") || name.ends_with(".sig")
            })
            .collect()
    }
    fn file(&self) -> PathBuf {
        self.path.join("updates/state.json")
    }
    fn value(&self) -> Value {
        serde_json::from_slice(&fs::read(self.file()).unwrap()).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        // Recursive cleanup is restricted to the exact native-test subtree we created.
        if let Ok(path) = self.path.canonicalize() {
            if path != self.root && path.starts_with(&self.root) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

fn release(version: &str) -> Value {
    release_sized(version, 1234)
}

fn release_sized(version: &str, size: usize) -> Value {
    let name = format!("Versora-{version}-win32-x64-setup.exe");
    json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":version.contains('-'),
        "html_url":format!("{OFFICIAL_RELEASE_URL}/tag/v{version}"),
        "body":format!("Versora {version}\r\n\r\n- Faster exports"),
        "assets":[{"name":name,"size":size,"state":"uploaded",
            "browser_download_url":format!("{OFFICIAL_RELEASE_URL}/download/v{version}/{name}")}]})
}

fn chunks(bytes: &[u8], size: usize, delay_ms: u64) -> FixtureSource {
    FixtureSource {
        chunks: bytes.chunks(size).map(<[u8]>::to_vec).collect(),
        delay: Duration::from_millis(delay_ms),
    }
}

fn clean() -> Value {
    json!({"busy":false,"unsaved":false,"pendingPersistence":false})
}

/// A trusted installed service with the 9.9.9 fixture discovered and nothing downloaded.
fn discovered(temp: &Temp) -> UpdateService {
    let service = temp.trusted("1.0.0");
    let checked = service
        .check_fixture(json!([release_sized("9.9.9", PACKAGE.len())]), 1000)
        .unwrap();
    assert_eq!(checked["status"], "available");
    service
}

#[test]
fn strict_versions_channels_and_highest_candidate_order() {
    for good in [
        "0.3.0-preview.1",
        "1.0.0",
        "1.0.0-alpha.0",
        "12.10.999-rc.a-1",
    ] {
        assert!(suite_updates::strict_version(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "v1.0.0",
        "01.0.0",
        "1.01.0",
        "1.0.00",
        "1.0.0-01",
        "1.0",
        "1.0.0+build",
        " 1.0.0",
        "1.0.0 ",
        "1.0.0-甲",
        "1.0.0-",
    ] {
        assert!(suite_updates::strict_version(bad).is_err(), "{bad:?}");
    }
    assert!(suite_updates::strict_version(&format!("1.0.0-{}", "a".repeat(128))).is_err());
    let rows = json!([
        release("1.9.0"),
        release("1.10.0-preview.2"),
        release("1.10.0"),
        release("1.10.0-preview.10")
    ]);
    assert_eq!(
        suite_updates::select_candidate(&rows, "stable", "0.3.0-preview.1", None)
            .unwrap()
            .unwrap()
            .version,
        "1.10.0"
    );
    assert_eq!(
        suite_updates::select_candidate(&rows, "preview", "0.3.0-preview.1", None)
            .unwrap()
            .unwrap()
            .version,
        "1.10.0-preview.10"
    );
    assert!(
        suite_updates::select_candidate(&rows, "preview", "0.3.0-preview.1", Some("1.10.0"))
            .unwrap()
            .is_none()
    );
    assert!(
        suite_updates::select_candidate(&rows, "stable", "1.10.0", None)
            .unwrap()
            .is_none()
    );
    assert!(suite_updates::select_candidate(&rows, "unknown", "1.0.0", None).is_err());
}

#[test]
fn metadata_identity_platform_and_asset_bounds_are_pinned() {
    let valid = release("1.0.0");
    for (pointer, value) in [
        ("/html_url", json!("https://github.com/attacker/Versora/releases/tag/v1.0.0")),
        ("/prerelease", json!(true)),
        ("/assets/0/browser_download_url", json!("https://github.com/mimateinn/Litora/releases/download/v1.0.0/Versora-1.0.0-win32-x64-setup.exe")),
        ("/assets/0/browser_download_url", json!("http://github.com/mimateinn/Versora/releases/download/v1.0.0/Versora-1.0.0-win32-x64-setup.exe")),
        ("/assets/0/size", json!(0)), ("/assets/0/size", json!(1_073_741_825u64)),
        ("/assets/0/state", json!("new")),
    ] {
        let mut wrong = valid.clone();
        *wrong.pointer_mut(pointer).unwrap() = value;
        assert!(suite_updates::select_candidate(&json!([wrong]), "stable", "0.3.0-preview.1", None).is_err(), "{pointer}");
    }
    let mut duplicates = valid.clone();
    duplicates["assets"]
        .as_array_mut()
        .unwrap()
        .push(valid["assets"][0].clone());
    assert!(suite_updates::select_candidate(
        &json!([duplicates]),
        "stable",
        "0.3.0-preview.1",
        None
    )
    .is_err());
    for other in [
        "Versora-v1.0.0-Windows-x64.zip",
        "Versora-1.0.0-win32-arm64-setup.exe",
        "Litora-1.0.0-win32-x64-setup.exe",
    ] {
        let mut row = valid.clone();
        row["assets"][0]["name"] = json!(other);
        assert!(
            suite_updates::select_candidate(&json!([row]), "stable", "0.3.0-preview.1", None)
                .unwrap()
                .is_none()
        );
    }
    let mut draft = valid;
    draft["draft"] = json!(true);
    assert!(
        suite_updates::select_candidate(&json!([draft]), "stable", "0.3.0-preview.1", None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        suite_updates::github_api_url(1).unwrap(),
        "https://api.github.com/repos/mimateinn/Versora/releases?per_page=100&page=1"
    );
    assert!(
        suite_updates::github_api_url(0).is_err() && suite_updates::github_api_url(21).is_err()
    );
}

#[test]
fn metadata_json_is_bounded_and_never_accepts_renderer_urls() {
    let mut at_cap = b"[]".to_vec();
    at_cap.resize(2 * 1024 * 1024, b' ');
    assert_eq!(
        suite_updates::parse_release_metadata(&at_cap).unwrap(),
        json!([])
    );
    at_cap.push(b' ');
    assert!(suite_updates::parse_release_metadata(&at_cap).is_err());
    for bad in [b"{}".as_slice(), b"[broken".as_slice(), &[0xff]] {
        assert!(suite_updates::parse_release_metadata(bad).is_err());
    }
}

#[test]
fn defaults_and_validated_preferences_persist_without_advancing_health() {
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    assert_eq!(
        service.get_preferences(),
        json!({"policy":"periodic","intervalHours":4,"channel":"stable","autoDownload":true})
    );
    assert!(service.is_check_due());
    let saved = service.set_preferences(json!({"policy":"periodic","intervalHours":168,"channel":"preview","autoDownload":true})).unwrap();
    assert_eq!(saved["active"], false);
    assert_eq!(saved["trust"], false);
    assert_eq!(saved["installCapabilities"], false);
    assert_eq!(saved["downloadCapability"], false);
    assert!(saved["highestHealthyVersion"].is_null());
    assert_eq!(
        temp.service("0.3.0-preview.1").get_preferences(),
        saved["preferences"]
    );
    let original = fs::read(temp.file()).unwrap();
    for bad in [
        json!({"policy":"automatic"}),
        json!({"intervalHours":0}),
        json!({"intervalHours":169}),
        json!({"intervalHours":1.5}),
        json!({"autoDownload":"true"}),
        json!({"channel":"nightly"}),
        json!({"url":"https://evil.example"}),
        json!({"path":"C:\\evil.exe"}),
        json!({"executable":"cmd.exe"}),
    ] {
        assert!(service.set_preferences(bad).is_err());
        assert_eq!(fs::read(temp.file()).unwrap(), original);
    }
    service.set_preferences(json!({"policy":"manual"})).unwrap();
    assert_eq!(service.seconds_until_next_check(), None);
}

#[test]
fn discovery_and_later_retain_metadata_but_never_ready_or_downloaded() {
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    service.set_preferences(json!({"policy":"manual"})).unwrap();
    let checked = service
        .check_fixture(json!([release("0.4.0")]), 1000)
        .unwrap();
    assert_eq!(checked["status"], "available");
    assert_eq!(checked["bytesReceived"], 0);
    assert_eq!(checked["packageDownloaded"], false);
    assert!(checked["highestHealthyVersion"].is_null());
    assert_eq!(service.later().unwrap()["deferred"], true);
    let restarted = temp.service("0.3.0-preview.1");
    assert_eq!(restarted.get_state()["deferred"], true);
    assert_eq!(restarted.get_state()["status"], "available");
    restarted
        .set_preferences(json!({"intervalHours":6}))
        .unwrap();
    assert_eq!(restarted.get_state()["deferred"], true);
    assert_eq!(restarted.get_state()["candidateVersion"], "0.4.0");
    restarted
        .set_preferences(json!({"channel":"preview"}))
        .unwrap();
    assert!(restarted.get_state()["candidate"].is_null());
    assert_eq!(restarted.get_state()["deferred"], false);
    assert!(restarted.later().is_err());
}

#[test]
fn healthy_ack_is_explicit_monotonic_and_channel_changes_never_lower_it() {
    let temp = Temp::new();
    let service = temp.service("1.3.0");
    service.set_preferences(json!({"policy":"manual"})).unwrap();
    assert!(service.get_state()["highestHealthyVersion"].is_null());
    assert_eq!(
        service.health_ack().unwrap()["highestHealthyVersion"],
        "1.3.0"
    );
    let older = temp.service("1.0.0");
    older.health_ack().unwrap();
    assert_eq!(temp.value()["highestHealthyVersion"], "1.3.0");
    older
        .set_preferences(json!({"channel":"preview","intervalHours":9}))
        .unwrap();
    assert_eq!(temp.value()["highestHealthyVersion"], "1.3.0");
    assert!(older
        .check_fixture(json!([release("1.2.0-preview.1")]), 2000)
        .unwrap()["candidate"]
        .is_null());
    // A stale writer cannot lower a marker already held in another live process/service.
    let mut stale = temp.value();
    stale["highestHealthyVersion"] = json!("1.0.0");
    fs::write(temp.file(), serde_json::to_vec(&stale).unwrap()).unwrap();
    service.set_preferences(json!({"intervalHours":7})).unwrap();
    assert_eq!(temp.value()["highestHealthyVersion"], "1.3.0");
}

#[test]
fn corrupt_state_fails_visibly_and_retains_exact_original_bytes() {
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    service.health_ack().unwrap();
    for bytes in [
        b"{broken".to_vec(),
        b"{}".to_vec(),
        vec![b' '; 256 * 1024 + 1],
    ] {
        fs::write(temp.file(), &bytes).unwrap();
        assert!(UpdateService::new(temp.path.clone(), "0.3.0-preview.1").is_err());
        assert!(service.health_ack().is_err());
        assert_eq!(fs::read(temp.file()).unwrap(), bytes);
    }
}

#[cfg(windows)]
#[test]
fn failed_atomic_replace_retains_preferences_and_cleans_owned_temporary_file() {
    struct ReadonlyGuard(PathBuf);
    impl Drop for ReadonlyGuard {
        fn drop(&mut self) {
            if let Ok(metadata) = fs::metadata(&self.0) {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                let _ = fs::set_permissions(&self.0, permissions);
            }
        }
    }
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    service.set_preferences(json!({"policy":"manual"})).unwrap();
    let original = fs::read(temp.file()).unwrap();
    let mut permissions = fs::metadata(temp.file()).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(temp.file(), permissions).unwrap();
    let readonly = ReadonlyGuard(temp.file());
    assert!(service
        .set_preferences(json!({"channel":"preview"}))
        .is_err());
    assert_eq!(fs::read(temp.file()).unwrap(), original);
    assert_eq!(service.get_preferences()["channel"], "stable");
    assert!(fs::read_dir(temp.path.join("updates"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".state-")));
    drop(readonly);
    assert!(service
        .set_preferences(json!({"channel":"preview"}))
        .is_ok());
}

#[test]
fn install_work_guards_run_before_the_truthful_manual_fallback_gate() {
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    for blocked in [
        json!({"busy":true,"unsaved":false,"pendingPersistence":false}),
        json!({"busy":false,"unsaved":true,"pendingPersistence":false}),
        json!({"busy":false,"unsaved":false,"pendingPersistence":true}),
        json!({"busy":false,"unsaved":false,"pendingPersistence":false,"url":"https://evil.example"}),
        json!({}),
    ] {
        assert!(service.request_install(blocked).is_err());
    }
    let result = service
        .request_install(json!({"busy":false,"unsaved":false,"pendingPersistence":false}))
        .unwrap();
    assert_eq!(result["installBlocked"], true);
    assert_eq!(result["appRemainsOpen"], true);
    assert_eq!(result["trust"], false);
    assert_eq!(result["installCapabilities"], false);
    assert_eq!(result["officialReleaseUrl"], service.official_release_url());
    assert!(result["capabilityReason"]
        .as_str()
        .unwrap()
        .contains("no reviewed publisher"));
    assert!(!temp.file().exists()); // no installer journal/package/health claim is fabricated.
}

#[tokio::test]
async fn cancellation_dedup_and_channel_change_wait_for_native_cleanup() {
    let temp = Temp::new();
    let service = Arc::new(temp.service("0.3.0-preview.1"));
    let task_service = service.clone();
    let task = tokio::spawn(async move {
        task_service
            .delayed_fixture(json!([release("0.4.0")]))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.get_state()["status"] != "checking" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = service.get_preferences();
    assert_eq!(service.check().await.unwrap()["active"], true); // dedup; makes no network call.
    assert!(service
        .set_preferences(json!({"channel":"preview"}))
        .is_err());
    assert_eq!(service.get_preferences(), before);
    let ended = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ended["active"], false);
    assert_eq!(ended["status"], "cancelled");
    assert_eq!(temp.value()["inProgress"], false);
    service
        .set_preferences(json!({"channel":"preview"}))
        .unwrap();
    assert_eq!(service.get_preferences()["channel"], "preview");
}

#[test]
fn bounded_backoff_and_explicit_retry_are_preserved_in_metadata_state() {
    assert_eq!(suite_updates::backoff_seconds(1), 60);
    assert_eq!(suite_updates::backoff_seconds(2), 120);
    assert_eq!(suite_updates::backoff_seconds(255), 3600);
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    service
        .set_preferences(json!({"policy":"periodic","intervalHours":2}))
        .unwrap();
    assert!(service
        .check_fixture(json!({"invalid":"array"}), 5000)
        .is_err());
    assert_eq!(service.get_state()["status"], "failed");
    assert_eq!(service.get_state()["nextCheckAt"], 65000);
    service.check_fixture(json!([]), 6000).unwrap(); // explicit retry ignores prior due time.
    assert_eq!(service.get_state()["status"], "idle");
    assert_eq!(service.get_state()["nextCheckAt"], 7_206_000);
}

#[test]
fn interrupted_check_recovers_as_cancelled_without_a_fake_ready_package() {
    let temp = Temp::new();
    let service = temp.service("0.3.0-preview.1");
    service.health_ack().unwrap();
    let mut state = temp.value();
    state["inProgress"] = json!(true);
    fs::write(temp.file(), serde_json::to_vec(&state).unwrap()).unwrap();
    let restarted = temp.service("0.3.0-preview.1");
    assert_eq!(restarted.get_state()["status"], "cancelled");
    assert_eq!(restarted.get_state()["packageDownloaded"], false);
    assert_eq!(restarted.get_state()["recoveryCapability"], false);
}

#[test]
fn update_lock_child() {
    let Ok(path) = std::env::var("VERSORA_UPDATE_LOCK_TEST_ROOT") else {
        return;
    };
    let child = UpdateService::new(PathBuf::from(path), "0.3.0-preview.1").unwrap();
    assert!(child.health_ack().unwrap_err().contains("Another process"));
}

#[tokio::test]
async fn actual_windows_process_lock_prevents_competing_metadata_writes() {
    let temp = Temp::new();
    let service = Arc::new(temp.service("0.3.0-preview.1"));
    let task_service = service.clone();
    let task = tokio::spawn(async move { task_service.delayed_fixture(json!([])).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.get_state()["status"] != "checking" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let path = temp.path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "update_lock_child", "--nocapture"])
            .env("VERSORA_UPDATE_LOCK_TEST_ROOT", path)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    service.cancel().unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(temp.service("0.3.0-preview.1").health_ack().is_ok());
}

#[test]
fn tauri_signer_signatures_verify_and_bind_the_version() {
    use suite_updates::{configured_public_key, decode_public_key, verify_package_bytes};
    let key = configured_public_key(TRUSTED_KEY).unwrap();
    verify_package_bytes(&key, PACKAGE, SIGNATURE, "9.9.9").unwrap();
    let mut tampered = PACKAGE.to_vec();
    tampered[100] ^= 1;
    assert!(verify_package_bytes(&key, &tampered, SIGNATURE, "9.9.9").is_err());
    assert!(verify_package_bytes(&key, &PACKAGE[1..], SIGNATURE, "9.9.9").is_err());
    let other = configured_public_key(OTHER_KEY).unwrap();
    assert!(verify_package_bytes(&other, PACKAGE, SIGNATURE, "9.9.9").is_err());
    assert!(verify_package_bytes(&key, PACKAGE, SIGNATURE_OTHER_KEY, "9.9.9").is_err());
    // The signed trusted comment must name exactly the announced version.
    assert!(verify_package_bytes(&key, PACKAGE, SIGNATURE, "9.9.10").is_err());
    assert!(
        verify_package_bytes(&key, PACKAGE, SIGNATURE_UNVERSIONED, "9.9.9")
            .unwrap_err()
            .contains("does not name the version")
    );
    assert!(
        verify_package_bytes(&key, PACKAGE, SIGNATURE_FOR_998, "9.9.9")
            .unwrap_err()
            .contains("different version")
    );
    for bad in ["", "not base64!", "dW50cnVzdGVkIGNvbW1lbnQ6IHg="] {
        assert!(verify_package_bytes(&key, PACKAGE, bad, "9.9.9").is_err());
        assert!(decode_public_key(bad).is_err());
    }
    assert_eq!(configured_public_key("# comment only\n\n   \n"), None);
    assert_eq!(
        configured_public_key("# comment\n  ABC  \n"),
        Some("ABC".to_owned())
    );
    // The committed trust anchor is either empty (no automatic updates) or a valid key.
    if let Some(committed) = configured_public_key(suite_updates::PUBLIC_KEY_FILE) {
        decode_public_key(&committed).unwrap();
    }
}

#[tokio::test]
async fn missing_invalid_key_or_unpackaged_build_keeps_execution_disabled() {
    let temp = Temp::new();
    let service = temp.service("1.0.0");
    service
        .check_fixture(json!([release_sized("9.9.9", PACKAGE.len())]), 1000)
        .unwrap();
    let state = service.get_state();
    assert_eq!(state["trust"], false);
    assert_eq!(state["downloadCapability"], false);
    assert_eq!(state["installCapabilities"], false);
    assert!(state["capabilityReason"]
        .as_str()
        .unwrap()
        .contains("no reviewed publisher"));
    assert!(service
        .download_fixture(chunks(PACKAGE, 4096, 0), Some(SIGNATURE))
        .await
        .unwrap_err()
        .contains("no reviewed publisher"));
    assert!(!service.auto_download_due());
    assert!(service.launch_installer(true).is_err());
    assert!(temp.leftovers().is_empty());

    let invalid = temp.configured("1.0.0", Some("bm90IGEga2V5"), true);
    assert_eq!(invalid.get_state()["trust"], false);
    assert_eq!(
        invalid.get_state()["capabilityReason"],
        suite_updates::INVALID_KEY_REASON
    );

    // B8: a development or portable copy never schedules or performs network checks.
    let unpackaged = temp.configured("1.0.0", Some(TRUSTED_KEY), false);
    assert_eq!(unpackaged.get_state()["installed"], false);
    assert_eq!(unpackaged.get_state()["downloadCapability"], false);
    assert_eq!(unpackaged.seconds_until_next_check(), None);
    assert!(unpackaged
        .check()
        .await
        .unwrap_err()
        .contains("installed Versora app"));
}

#[tokio::test]
async fn verified_download_is_ready_restarts_ready_and_approves_install() {
    let temp = Temp::new();
    let service = discovered(&temp);
    assert!(service.auto_download_due());
    let state = service
        .download_fixture(chunks(PACKAGE, 5000, 0), Some(SIGNATURE))
        .await
        .unwrap();
    assert_eq!(state["status"], "ready");
    assert_eq!(state["packageDownloaded"], true);
    assert_eq!(state["bytesReceived"], PACKAGE.len());
    assert_eq!(state["totalBytes"], PACKAGE.len());
    assert_eq!(state["installOnQuit"], true);
    assert_eq!(
        fs::read(temp.updates().join("Versora-9.9.9-win32-x64-setup.exe")).unwrap(),
        PACKAGE
    );
    assert!(!temp.leftovers().iter().any(|name| name.ends_with(".part")));
    assert!(!service.auto_download_due());
    assert!(service.install_on_quit_ready());
    service.verify_ready_package().unwrap();

    let restarted = temp.trusted("1.0.0");
    assert_eq!(restarted.get_state()["status"], "ready");
    // Busy or unsaved work needs the user's explicit confirmation; pending saves always refuse.
    let mut busy = clean();
    busy["busy"] = json!(true);
    assert!(restarted.request_install(busy.clone()).is_err());
    busy["confirmed"] = json!(true);
    assert_eq!(
        restarted.request_install(busy).unwrap()["installApproved"],
        true
    );
    let mut pending = clean();
    pending["pendingPersistence"] = json!(true);
    pending["confirmed"] = json!(true);
    assert!(restarted.request_install(pending).is_err());
    let approved = restarted.request_install(clean()).unwrap();
    assert_eq!(approved["installApproved"], true);
    assert_eq!(approved["appRemainsOpen"], false);
    // Turning automatic updates off keeps the package but no longer installs it on quit.
    restarted
        .set_preferences(json!({"autoDownload":false}))
        .unwrap();
    assert_eq!(restarted.get_state()["status"], "ready");
    assert!(!restarted.install_on_quit_ready());
}

#[tokio::test]
async fn invalid_or_missing_signatures_are_discarded_and_never_ready() {
    let temp = Temp::new();
    let service = discovered(&temp);
    for signature in [
        Some(SIGNATURE_OTHER_KEY),
        Some(SIGNATURE_UNVERSIONED),
        Some(SIGNATURE_FOR_998),
        Some("garbage"),
        None,
    ] {
        let error = service
            .download_fixture(chunks(PACKAGE, 4096, 0), signature)
            .await
            .unwrap_err();
        assert!(!error.is_empty());
        let state = service.get_state();
        assert_eq!(state["status"], "failed");
        assert_eq!(state["packageDownloaded"], false);
        assert!(temp.leftovers().is_empty(), "{:?}", temp.leftovers());
        assert!(service.verify_ready_package().is_err());
        assert!(service.request_install(clean()).is_err());
    }
    let mut tampered = PACKAGE.to_vec();
    tampered[0] ^= 0xff;
    assert!(service
        .download_fixture(chunks(&tampered, 4096, 0), Some(SIGNATURE))
        .await
        .unwrap_err()
        .contains("signature is invalid"));
    assert!(temp.leftovers().is_empty());
}

#[tokio::test]
async fn download_size_is_capped_by_release_metadata() {
    let temp = Temp::new();
    let service = discovered(&temp);
    let mut oversized = PACKAGE.to_vec();
    oversized.push(0);
    assert!(service
        .download_fixture(chunks(&oversized, 4096, 0), Some(SIGNATURE))
        .await
        .unwrap_err()
        .contains("larger than its release metadata"));
    assert!(temp.leftovers().is_empty());
    assert!(service
        .download_fixture(
            chunks(&PACKAGE[..PACKAGE.len() - 1], 4096, 0),
            Some(SIGNATURE)
        )
        .await
        .unwrap_err()
        .contains("ended early"));
    assert!(temp.leftovers().is_empty());
    assert_eq!(service.get_state()["status"], "failed");
}

#[tokio::test]
async fn cancelling_a_download_removes_the_partial_file() {
    let temp = Temp::new();
    let service = Arc::new(discovered(&temp));
    let task_service = service.clone();
    let task = tokio::spawn(async move {
        task_service
            .download_fixture(chunks(PACKAGE, 1024, 50), Some(SIGNATURE))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.get_state()["bytesReceived"].as_u64().unwrap_or(0) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(service.get_state()["status"], "downloading");
    assert!(temp.leftovers().iter().any(|name| name.ends_with(".part")));
    assert!(service
        .set_preferences(json!({"channel":"preview"}))
        .is_err());
    service.cancel().unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ended["status"], "cancelled");
    assert_eq!(ended["active"], false);
    assert_eq!(ended["packageDownloaded"], false);
    assert!(temp.leftovers().is_empty(), "{:?}", temp.leftovers());
    // A later download of the same candidate still completes.
    assert_eq!(
        service
            .download_fixture(chunks(PACKAGE, 8192, 0), Some(SIGNATURE))
            .await
            .unwrap()["status"],
        "ready"
    );
}

#[tokio::test]
async fn install_is_refused_when_the_package_changes_after_verification() {
    let temp = Temp::new();
    let service = discovered(&temp);
    service
        .download_fixture(chunks(PACKAGE, 8192, 0), Some(SIGNATURE))
        .await
        .unwrap();
    let package = temp.updates().join("Versora-9.9.9-win32-x64-setup.exe");
    let mut tampered = PACKAGE.to_vec();
    tampered[1] ^= 1;
    fs::write(&package, &tampered).unwrap();
    assert!(service.verify_ready_package().is_err());
    assert!(service.request_install(clean()).is_err());
    assert_eq!(service.get_state()["status"], "available");
    assert!(service.launch_installer(true).is_err());
    assert!(!temp.trusted("1.0.0").install_on_quit_ready());
    // A missing signature file is refused just the same.
    fs::write(&package, PACKAGE).unwrap();
    fs::remove_file(temp.updates().join("Versora-9.9.9-win32-x64-setup.exe.sig")).unwrap();
    assert!(service.verify_ready_package().is_err());
    assert!(service.launch_installer(false).is_err());
}

#[test]
fn release_notes_are_bounded_plain_text() {
    let notes = suite_updates::bound_release_notes(
        "1.2.0",
        "<b>Bold</b>\r\n\r\n\r\n\r\nline\u{0007}two\u{202E}three\ttab",
    );
    assert_eq!(notes.text, "<b>Bold</b>\n\nlinetwothree\ttab");
    assert!(!notes.truncated);
    let long = suite_updates::bound_release_notes("1.2.0", &"字".repeat(10_000));
    assert!(long.truncated);
    assert!(long.text.len() <= suite_updates::NOTES_CAP);
    assert!(long.text.chars().all(|c| c == '字'));
    let releases = json!([release("1.2.0"), {"tag_name":"v1.3.0"}]);
    assert_eq!(
        suite_updates::release_notes(&releases, "1.2.0")
            .unwrap()
            .text,
        "Versora 1.2.0\n\n- Faster exports"
    );
    assert_eq!(
        suite_updates::release_notes(&releases, "1.3.0")
            .unwrap()
            .text,
        ""
    );
    assert!(suite_updates::release_notes(&releases, "9.0.0").is_none());
    let temp = Temp::new();
    let service = temp.service("1.0.0");
    let state = service.check_fixture(releases, 1000).unwrap();
    assert_eq!(state["releaseNotes"]["version"], "1.2.0");
    assert_eq!(state["releaseNotes"]["truncated"], false);
}

#[test]
fn legacy_default_preferences_migrate_once_and_explicit_choices_stay() {
    let legacy = |preferences: Value| {
        json!({"schema":1,"preferences":preferences,"highestHealthyVersion":"0.3.0-preview.1",
            "candidate":null,"deferred":false,"lastCheckAt":null,"nextCheckAt":null,"failures":0,"inProgress":false})
    };
    let temp = Temp::new();
    fs::create_dir_all(temp.updates()).unwrap();
    fs::write(
        temp.file(),
        serde_json::to_vec(&legacy(
            json!({"policy":"startup","intervalHours":24,"channel":"stable","autoDownload":false}),
        ))
        .unwrap(),
    )
    .unwrap();
    let service = temp.service("0.3.0");
    assert_eq!(
        service.get_preferences(),
        json!({"policy":"periodic","intervalHours":4,"channel":"stable","autoDownload":true})
    );
    assert_eq!(temp.value()["preferences"], service.get_preferences());
    // Once migrated, the same values chosen again explicitly are kept.
    service
        .set_preferences(json!({"policy":"startup","intervalHours":24}))
        .unwrap();
    assert_eq!(temp.service("0.3.0").get_preferences()["policy"], "startup");

    let explicit = Temp::new();
    fs::create_dir_all(explicit.updates()).unwrap();
    fs::write(
        explicit.file(),
        serde_json::to_vec(&legacy(
            json!({"policy":"manual","intervalHours":24,"channel":"preview","autoDownload":false}),
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        explicit.service("0.3.0").get_preferences(),
        json!({"policy":"manual","intervalHours":24,"channel":"preview","autoDownload":true})
    );
    // The state file keeps the schema an older build can still read.
    let saved = explicit.value();
    assert_eq!(saved["schema"], 1);
    assert!(saved.get("lastSeenVersion").is_none());
}

#[test]
fn first_launch_after_an_update_reports_it_once() {
    let temp = Temp::new();
    let old = temp.service("1.0.0");
    old.health_ack().unwrap();
    assert!(old.get_state()["justUpdated"].is_null());
    let mut row = release("1.1.0");
    row["body"] = json!("Faster exports");
    old.check_fixture(json!([row]), 1000).unwrap();
    drop(old);
    let new = temp.service("1.1.0");
    let state = new.health_ack().unwrap();
    assert_eq!(state["justUpdated"]["from"], "1.0.0");
    assert_eq!(state["justUpdated"]["to"], "1.1.0");
    assert_eq!(
        state["justUpdated"]["releaseNotes"]["text"],
        "Faster exports"
    );
    assert_eq!(state["releaseNotes"]["version"], "1.1.0");
    assert!(state["candidate"].is_null());
    drop(new);
    assert!(temp.service("1.1.0").health_ack().unwrap()["justUpdated"].is_null());
    // A profile written by a build without updater.json still reports the update.
    let legacy = Temp::new();
    legacy.service("0.3.0-preview.1").health_ack().unwrap();
    fs::remove_file(legacy.updates().join("updater.json")).unwrap();
    assert_eq!(
        legacy.service("0.3.0").health_ack().unwrap()["justUpdated"]["from"],
        "0.3.0-preview.1"
    );
    // Starting an older build never claims an update.
    assert!(legacy.service("0.2.0").health_ack().unwrap()["justUpdated"].is_null());
}

#[tokio::test]
async fn health_ack_prunes_packages_that_are_no_longer_candidates() {
    let temp = Temp::new();
    let service = discovered(&temp);
    service
        .download_fixture(chunks(PACKAGE, 8192, 0), Some(SIGNATURE))
        .await
        .unwrap();
    fs::write(temp.updates().join(".download-1-1.part"), b"stale").unwrap();
    fs::write(temp.updates().join("unrelated.txt"), b"kept").unwrap();
    drop(service);
    // After the update, 9.9.9 is the running version: its package is obsolete.
    let updated = temp.trusted("9.9.9");
    updated.health_ack().unwrap();
    assert!(temp.leftovers().is_empty(), "{:?}", temp.leftovers());
    assert!(temp.updates().join("unrelated.txt").exists());
}

/// Release workflow check: verifies a freshly signed installer against the trust anchor
/// compiled into this build. Skipped unless VERSORA_RELEASE_ARTIFACT is set.
#[test]
fn release_artifact_verifies_with_the_committed_key() {
    let Ok(artifact) = std::env::var("VERSORA_RELEASE_ARTIFACT") else {
        return;
    };
    let version = std::env::var("VERSORA_RELEASE_VERSION").unwrap();
    let key = suite_updates::configured_public_key(suite_updates::PUBLIC_KEY_FILE)
        .expect("updater-public-key.txt has no key");
    let signature = fs::read_to_string(format!("{artifact}.sig")).unwrap();
    suite_updates::verify_package_bytes(&key, &fs::read(&artifact).unwrap(), &signature, &version)
        .unwrap();
}

/// The verification handle denies write/delete sharing but must still let Windows start
/// the installer from the same file.
#[cfg(windows)]
#[test]
fn verified_package_handle_still_allows_the_installer_to_start() {
    let temp = Temp::new();
    let source = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32\\whoami.exe");
    let copy = temp.path.join("whoami-copy.exe");
    fs::copy(&source, &copy).unwrap();
    let handle = suite_updates::open_package_for_launch(&copy).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&copy).is_err());
    assert!(fs::remove_file(&copy).is_err());
    let status = std::process::Command::new(&copy)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    drop(handle);
    fs::remove_file(&copy).unwrap();
}
