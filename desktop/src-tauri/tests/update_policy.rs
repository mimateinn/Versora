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
use suite_updates::{UpdateService, OFFICIAL_RELEASE_URL};
static NEXT: AtomicU64 = AtomicU64::new(0);

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
    fn service(&self, version: &str) -> UpdateService {
        UpdateService::new(self.path.clone(), version).unwrap()
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
    let name = format!("Versora-{version}-win32-x64-setup.exe");
    json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":version.contains('-'),
        "html_url":format!("{OFFICIAL_RELEASE_URL}/tag/v{version}"),
        "assets":[{"name":name,"size":1234,"state":"uploaded",
            "browser_download_url":format!("{OFFICIAL_RELEASE_URL}/download/v{version}/{name}")}]})
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
        json!({"policy":"startup","intervalHours":24,"channel":"stable","autoDownload":false})
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
