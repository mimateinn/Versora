//! Native filesystem/format/job acceptance with the product's explicitly opt-in Demo transport.
//! Demo only tags text. Paid provider translation quality and live CLI containment are NOT_RUN.
#[path = "../src/jobs.rs"]
mod jobs;

use jobs::{JobManager, JobSnapshot, SelectedInput, TranslationRequest};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

// jobs.rs calls the binary crate's preview helper. This harness only records byte counts;
// preview rendering/content parity is deliberately outside these native job tests.
fn preview_text(source: &[u8], translated: Option<&[u8]>) -> Value {
    json!({"harnessOnly": true, "sourceBytes": source.len(), "translatedBytes": translated.map(<[u8]>::len)})
}

static NEXT: AtomicU64 = AtomicU64::new(1);
static ENV_LOCK: Mutex<()> = Mutex::new(());
struct DemoEnvironment {
    _guard: MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<OsString>)>,
}
impl DemoEnvironment {
    fn new(enabled: bool) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous = ["VERSORA_DEMO", "SFTS_DEMO"]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        std::env::remove_var("VERSORA_DEMO");
        std::env::remove_var("SFTS_DEMO");
        if enabled {
            std::env::set_var("VERSORA_DEMO", "1");
        }
        Self {
            _guard: guard,
            previous,
        }
    }
}
impl Drop for DemoEnvironment {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

struct TestDirectory {
    root: PathBuf,
    parent: PathBuf,
}
// Tauri owns a process-global runtime; a failed async test does not stop a detached
// job. Drop this guard before Temp/Demo guards, including when assertions unwind.
struct ManagedJob(Arc<JobManager>);
impl ManagedJob {
    fn new() -> Self {
        Self(Arc::new(JobManager::new()))
    }
}
impl std::ops::Deref for ManagedJob {
    type Target = Arc<JobManager>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for ManagedJob {
    fn drop(&mut self) {
        if let Some(id) = self.0.active() {
            let _ = self.0.cancel(&id);
            let deadline = Instant::now() + Duration::from_secs(15);
            while self.0.active().is_some() {
                if Instant::now() >= deadline {
                    eprintln!("Harness cancellation did not drain job {id}; retaining Temp fixtures and failing the test process before cleanup.");
                    std::process::exit(1);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl TestDirectory {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let root = parent.join(format!(
            "versora-job-pipeline-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, parent }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        // Delete only the generated, direct child of the resolved Temp root.
        if self.root.parent() == Some(self.parent.as_path())
            && self
                .root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("versora-job-pipeline-"))
        {
            if let Ok(resolved) = self.root.canonicalize() {
                if resolved.parent() == Some(self.parent.as_path()) {
                    let _ = fs::remove_dir_all(resolved);
                }
            }
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn snapshot_sources(root: &Path) -> BTreeMap<PathBuf, (String, Vec<u8>)> {
    let mut result = BTreeMap::new();
    fn visit(root: &Path, result: &mut BTreeMap<PathBuf, (String, Vec<u8>)>) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, result);
            } else {
                let bytes = fs::read(&path).unwrap();
                result.insert(path, (digest(&bytes), bytes));
            }
        }
    }
    visit(root, &mut result);
    result
}
fn assert_sources_unchanged(before: &BTreeMap<PathBuf, (String, Vec<u8>)>) {
    for (path, (hash, original)) in before {
        let bytes = fs::read(path).unwrap();
        assert_eq!(
            &digest(&bytes),
            hash,
            "input SHA256 changed: {}",
            path.display()
        );
        assert_eq!(&bytes, original, "input bytes changed: {}", path.display());
    }
}
fn selected(path: &Path) -> SelectedInput {
    let path = path.canonicalize().unwrap();
    let metadata = fs::metadata(&path).unwrap();
    SelectedInput {
        path: path.to_string_lossy().into(),
        name: path.file_name().unwrap().to_string_lossy().into(),
        size: metadata.len(),
        kind: if metadata.is_dir() {
            "folder"
        } else if path.extension().is_some_and(|e| e == "zip") {
            "zip"
        } else {
            "file"
        }
        .into(),
    }
}
fn request() -> TranslationRequest {
    TranslationRequest {
        source_language: Some("en".into()),
        target_language: "zh-Hant".into(),
        provider_id: "demo".into(),
        purpose_id: "general".into(),
        model: Some("offline-demo".into()),
        project: Some("test-project".into()),
        mode: "document".into(),
        ..Default::default()
    }
}
fn demo() -> Vec<versora_engine::providers::ProviderConfig> {
    vec![versora_engine::providers::ProviderConfig {
        id: "demo".into(),
        ..Default::default()
    }]
}
async fn start(
    manager: &Arc<JobManager>,
    selected: &[SelectedInput],
    output: &Path,
    concurrency: usize,
) -> String {
    manager
        .start(
            request(),
            selected,
            output,
            demo(),
            "Offline pipeline acceptance. Preserve structural markers.".into(),
            concurrency,
            1,
            None,
        )
        .await
        .unwrap()
}
async fn terminal(manager: &JobManager, id: &str) -> JobSnapshot {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let snap = manager.snapshot(id).unwrap();
        if matches!(snap.status.as_str(), "done" | "error" | "stopped")
            && manager.active().is_none()
        {
            // Every requested transport in this harness is the real opt-in Demo.
            // Provenance describes saved outputs, including partial cancellation;
            // failures, skipped inputs and unsaved cancelled work must stay false.
            assert_eq!(
                snap.used_demo,
                snap.saved > 0,
                "incorrect job Demo provenance"
            );
            for file in &snap.files {
                assert_eq!(
                    file.used_demo,
                    file.status == "saved",
                    "incorrect Demo provenance for {} ({})",
                    file.relative,
                    file.status
                );
            }
            assert_eq!(
                serde_json::to_value(&snap).unwrap()["usedDemo"].as_bool(),
                Some(snap.saved > 0)
            );
            return snap;
        }
        assert!(
            Instant::now() < until,
            "job {id} did not finish: {} done={}/{} saved={} failed={}",
            snap.status,
            snap.done,
            snap.total,
            snap.saved,
            snap.failed
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn zip_members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().to_string();
        assert!(
            entry.enclosed_name().is_some(),
            "unsafe exported ZIP member {name}"
        );
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents).unwrap();
        assert!(
            entries.insert(name, contents).is_none(),
            "duplicate exported ZIP member"
        );
    }
    entries
}
fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn assert_export_matches_disk(manager: &JobManager, id: &str) -> BTreeMap<String, Vec<u8>> {
    let snapshot = manager.snapshot(id).unwrap();
    let (name, bytes) = manager.export_bytes(id, true).unwrap();
    assert!(name.ends_with(".zip"));
    let members = zip_members(&bytes);
    assert_eq!(members.len(), snapshot.saved);
    for file in snapshot.files.iter().filter(|f| f.status == "saved") {
        let output = Path::new(file.output.as_ref().unwrap());
        let relative = output
            .strip_prefix(&snapshot.output_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let disk = fs::read(output).unwrap();
        assert_eq!(digest(&members[&relative]), digest(&disk));
        assert_eq!(members[&relative], disk);
        assert!(manager.owned_output(output));
    }
    members
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folder_mixed_formats_failures_and_exports_preserve_all_original_hashes() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    directory.write("input/sub/manual.txt", b"Hello world\r\n  Second line\r\n");
    directory.write(
        "input/sub/menu.json",
        br#"{ "title": "Hello world!", "id": "menu_button", "count": 7 }"#,
    );
    directory.write(
        "input/cues.srt",
        b"1\r\n00:00:01,000 --> 00:00:02,000\r\nHello world!\r\n\r\n",
    );
    directory.write("input/malformed.json", b"{ malformed JSON }");
    directory.write("input/unsupported.bin", &[0, 255]);
    directory.write(
        "input/.git/private.txt",
        b"Hidden test content must be skipped.",
    );
    directory.write(
        "input/node_modules/fixture.txt",
        b"Dependency test content must be skipped.",
    );
    let before = snapshot_sources(&directory.path("input"));
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("input"))],
        &directory.path("outputs"),
        3,
    )
    .await;
    let snapshot = terminal(&manager, &id).await;
    assert_eq!(snapshot.status, "done");
    assert_eq!(
        (
            snapshot.total,
            snapshot.done,
            snapshot.saved,
            snapshot.failed,
            snapshot.skipped
        ),
        (5, 5, 3, 1, 1)
    );
    assert!(snapshot
        .files
        .iter()
        .all(|f| !f.relative.contains(".git") && !f.relative.contains("node_modules")));
    assert!(snapshot.files.iter().any(|f| f.relative == "malformed.json"
        && f.status == "failed"
        && f.error.as_ref().unwrap().contains("Malformed JSON")));
    let exports = assert_export_matches_disk(&manager, &id);
    assert!(exports.contains_key("sub/manual.zh-Hant.txt"));
    assert!(exports.contains_key("sub/menu.zh-Hant.json"));
    let json: Value = serde_json::from_slice(&exports["sub/menu.zh-Hant.json"]).unwrap();
    assert_eq!(json["title"], "[zh-Hant] Hello world!");
    assert_eq!(json["id"], "menu_button");
    assert_eq!(json["count"], 7);
    let srt = std::str::from_utf8(&exports["cues.zh-Hant.srt"]).unwrap();
    assert!(srt.contains("00:00:01,000 --> 00:00:02,000\r\n[zh-Hant] Hello world!"));
    assert_sources_unchanged(&before);
    assert!(!manager.owned_output(&directory.path("input/sub/manual.txt")));
    assert!(manager
        .preview(&id, Some("unselected/private.txt"))
        .is_err());
    assert!(manager.preview(&id, None).unwrap()["harnessOnly"]
        .as_bool()
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exports_reject_changed_output_sizes_and_parent_junction_replacements() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    directory.write("input/first.txt", b"First real export input.");
    directory.write("input/second.txt", b"Second real export input.");
    let originals = snapshot_sources(&directory.path("input"));
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("input"))],
        &directory.path("outputs"),
        1,
    )
    .await;
    let snapshot = terminal(&manager, &id).await;
    assert_eq!(snapshot.saved, 2);
    assert_export_matches_disk(&manager, &id);
    let outputs: Vec<_> = snapshot
        .files
        .iter()
        .map(|row| PathBuf::from(row.output.as_ref().unwrap()))
        .collect();
    let original_outputs: Vec<_> = outputs.iter().map(|path| fs::read(path).unwrap()).collect();
    fs::OpenOptions::new()
        .write(true)
        .open(&outputs[0])
        .unwrap()
        .set_len(80_000_001)
        .unwrap();
    assert!(manager
        .export_bytes(&id, true)
        .unwrap_err()
        .contains("limit"));
    fs::write(&outputs[0], &original_outputs[0]).unwrap();
    for path in &outputs {
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(40_000_001)
            .unwrap();
    }
    assert!(manager
        .export_bytes(&id, true)
        .unwrap_err()
        .contains("limit"));
    for (path, bytes) in outputs.iter().zip(&original_outputs) {
        fs::write(path, bytes).unwrap();
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let original_parent = directory.path("outputs");
        let preserved_parent = directory.path("outputs-preserved");
        let replacement_parent = directory.path("replacement");
        let job_name = Path::new(&snapshot.output_dir).file_name().unwrap();
        let replacement_root = replacement_parent.join(job_name);
        fs::create_dir_all(&replacement_root).unwrap();
        for output in &outputs {
            fs::write(
                replacement_root.join(output.file_name().unwrap()),
                b"Unregistered replacement content.",
            )
            .unwrap();
        }
        fs::rename(&original_parent, &preserved_parent).unwrap();
        let created = std::process::Command::new("cmd.exe")
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&original_parent)
            .arg(&replacement_parent)
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "Actual junction replacement must succeed."
        );
        let result = manager.export_bytes(&id, true);
        // Remove only the actual junction before any assertion can unwind fixture cleanup.
        fs::remove_dir(&original_parent).unwrap();
        fs::rename(&preserved_parent, &original_parent).unwrap();
        assert!(result.unwrap_err().contains("registered path"));
        assert!(replacement_root.is_dir());
    }
    assert_export_matches_disk(&manager, &id);
    assert_eq!(snapshot_sources(&directory.path("input")), originals);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_subset_and_unregistered_paths_are_scoped_before_output_creation() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let first = directory.write("inputs/first.txt", b"First original document.");
    let second = directory.write("inputs/second.txt", b"Second original document.");
    let unregistered = directory.write(
        "inputs/not-picked.txt",
        b"Not selected through the native picker.",
    );
    let selection = [selected(&first), selected(&second)];
    let manager = ManagedJob::new();
    let mut restricted = request();
    restricted.input_paths = vec![unregistered
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into()];
    let error = manager
        .start(
            restricted,
            &selection,
            &directory.path("rejected-output"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("not selected"));
    assert!(manager.active().is_none());
    assert!(!directory.path("rejected-output").exists());
    let before = snapshot_sources(&directory.path("inputs"));
    let mut restricted = request();
    restricted.input_paths = vec![selection[1].path.clone()];
    let id = manager
        .start(
            restricted,
            &selection,
            &directory.path("accepted-output"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap();
    let snapshot = terminal(&manager, &id).await;
    assert_eq!((snapshot.total, snapshot.saved), (1, 1));
    assert_eq!(snapshot.files[0].name, "second.txt");
    let (name, bytes) = manager.export_bytes(&id, false).unwrap();
    assert_eq!(name, "second.zh-Hant.txt");
    let disk = fs::read(snapshot.files[0].output.as_ref().unwrap()).unwrap();
    assert_eq!(bytes, disk);
    assert_eq!(digest(&bytes), digest(&disk));
    assert_sources_unchanged(&before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_batch_preserves_subfolders_and_rejects_unsafe_and_unsupported_members() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let bytes = package(&[
        ("nested/manual.txt", b"ZIP document text."),
        (
            "nested/menu.json",
            br#"{"caption":"ZIP string value!","number":12}"#,
        ),
        ("nested/bad.json", b"{"),
        ("../escape.txt", b"Outside root must not be written."),
        ("NUL.txt", b"Windows device must not be written."),
        ("images/asset.bin", b"Not a supported document."),
    ]);
    let source = directory.write("inputs/source.zip", &bytes);
    let hash = digest(&bytes);
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&source)],
        &directory.path("outputs"),
        3,
    )
    .await;
    let snapshot = terminal(&manager, &id).await;
    assert_eq!(
        (
            snapshot.total,
            snapshot.saved,
            snapshot.failed,
            snapshot.skipped
        ),
        (6, 2, 1, 3)
    );
    assert_eq!(
        snapshot
            .files
            .iter()
            .filter(|f| f.error.as_deref() == Some("Unsafe ZIP entry path."))
            .count(),
        2
    );
    let exports = assert_export_matches_disk(&manager, &id);
    assert_eq!(
        exports.keys().cloned().collect::<Vec<_>>(),
        ["nested/manual.zh-Hant.txt", "nested/menu.zh-Hant.json"]
    );
    assert!(!directory.path("escape.txt").exists());
    assert_eq!(digest(&fs::read(source).unwrap()), hash);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_office_and_localization_outputs_survive_the_full_job_pipeline() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    directory.write(
        "inputs/document.docx",
        include_bytes!("../../crates/versora-engine/tests/fixtures/document.docx"),
    );
    directory.write(
        "inputs/workbook.xlsx",
        include_bytes!("../../crates/versora-engine/tests/fixtures/workbook.xlsx"),
    );
    directory.write(
        "inputs/localization.xlf",
        include_bytes!("../../crates/versora-engine/tests/fixtures/localization.xlf"),
    );
    directory.write(
        "inputs/messages.po",
        include_bytes!("../../crates/versora-engine/tests/fixtures/messages.po"),
    );
    let before = snapshot_sources(&directory.path("inputs"));
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("inputs"))],
        &directory.path("outputs"),
        3,
    )
    .await;
    let snapshot = terminal(&manager, &id).await;
    assert_eq!(
        (
            snapshot.total,
            snapshot.saved,
            snapshot.failed,
            snapshot.skipped
        ),
        (4, 4, 0, 0),
        "{}",
        serde_json::to_string(&snapshot).unwrap()
    );
    for file in &snapshot.files {
        let output = fs::read(file.output.as_ref().unwrap()).unwrap();
        let extension = Path::new(&file.name).extension().unwrap().to_str().unwrap();
        let document = versora_engine::formats::extract(&output, extension, "document").unwrap();
        if extension != "xlf" {
            assert!(
                document
                    .units
                    .iter()
                    .all(|unit| unit.starts_with("[zh-Hant] ")),
                "{}: {:?}",
                file.name,
                document.units
            );
        }
        if matches!(extension, "docx" | "xlsx") {
            let original = fs::read(&file.input).unwrap();
            let original_members = zip_members(&original);
            let output_members = zip_members(&output);
            assert_eq!(
                original_members.keys().collect::<Vec<_>>(),
                output_members.keys().collect::<Vec<_>>()
            );
            for (name, bytes) in original_members {
                if !matches!(
                    name.as_str(),
                    "word/document.xml" | "xl/sharedStrings.xml" | "xl/worksheets/sheet1.xml"
                ) {
                    assert_eq!(output_members[&name], bytes, "{name}");
                }
            }
        } else if extension == "xlf" {
            let text = std::str::from_utf8(&output).unwrap();
            assert!(text.contains("<x:target state=\"needs-translation\">[zh-Hant] Hello <x:g id=\"b\">world</x:g><x:x id=\"line\"/></x:target>"));
            assert!(text.contains("<x:source>Click here!</x:source>"));
        }
    }
    assert_export_matches_disk(&manager, &id);
    assert_sources_unchanged(&before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opt_in_is_required_and_retry_only_processes_unfinished_files() {
    let demo_guard = DemoEnvironment::new(false);
    let directory = TestDirectory::new();
    let input = directory.write(
        "inputs/manual.txt",
        b"Demo cannot be used without explicit opt-in.",
    );
    let before = snapshot_sources(&directory.path("inputs"));
    let manager = ManagedJob::new();
    let id = start(&manager, &[selected(&input)], &directory.path("outputs"), 1).await;
    let failed = terminal(&manager, &id).await;
    assert_eq!(failed.status, "error");
    assert_eq!((failed.saved, failed.failed), (0, 1));
    assert!(failed.files.iter().all(|file| file.output.is_none()));
    assert!(failed.files[0]
        .error
        .as_ref()
        .unwrap()
        .contains("explicitly enabled"));
    let preserved = manager.retry_request(&id).unwrap();
    assert_eq!(preserved.source_language.as_deref(), Some("en"));
    assert_eq!(preserved.target_language, "zh-Hant");
    assert_eq!(preserved.purpose_id, "general");
    assert_eq!(preserved.provider_id, "demo");
    assert_eq!(preserved.model.as_deref(), Some("offline-demo"));
    assert_eq!(preserved.project.as_deref(), Some("test-project"));
    assert!(preserved.output_dir.is_none());
    assert!(manager.export_bytes(&id, false).is_err());
    std::env::set_var("VERSORA_DEMO", "1");
    let retry = manager
        .restart(
            &id,
            &directory.path("outputs"),
            demo(),
            "Offline acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap();
    let finished = terminal(&manager, &retry).await;
    assert_ne!(finished.output_dir, failed.output_dir);
    assert_eq!((finished.total, finished.saved, finished.failed), (1, 1, 0));
    assert!(
        manager.snapshot(&id).is_err(),
        "Only the current job report is retained; old ZIP buffers must be released."
    );
    assert!(manager
        .restart(
            &retry,
            &directory.path("outputs"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None
        )
        .await
        .unwrap_err()
        .contains("No unfinished"));
    assert_sources_unchanged(&before);
    drop(demo_guard);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_keeps_observed_completed_files_and_retry_preserves_them() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    for index in 0..8 {
        directory.write(
            &format!("inputs/item-{index:02}.txt"),
            format!("Actual offline Demo file number {index}.").as_bytes(),
        );
    }
    let before = snapshot_sources(&directory.path("inputs"));
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("inputs"))],
        &directory.path("outputs"),
        1,
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snap = manager.snapshot(&id).unwrap();
        if snap.saved > 0 && snap.saved < snap.total {
            break;
        }
        assert_eq!(
            snap.status, "running",
            "job completed before partial cancellation could be observed"
        );
        assert!(
            Instant::now() < deadline,
            "no completed Demo output observed before cancellation"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(manager
        .restart(
            &id,
            &directory.path("outputs"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None
        )
        .await
        .unwrap_err()
        .contains("Wait"));
    manager.cancel(&id).unwrap();
    let stopped = terminal(&manager, &id).await;
    assert_eq!(stopped.status, "stopped");
    assert!(stopped.saved > 0 && stopped.saved < 8);
    assert_eq!(stopped.failed, 0);
    let cancelled = stopped
        .files
        .iter()
        .filter(|f| f.status == "cancelled")
        .count();
    assert_eq!(stopped.saved + cancelled, 8);
    assert_eq!(stopped.done, 8);
    let completed: BTreeMap<_, _> = stopped
        .files
        .iter()
        .filter(|f| f.status == "saved")
        .map(|file| {
            let path = PathBuf::from(file.output.as_ref().unwrap());
            let bytes = fs::read(&path).unwrap();
            (path, (digest(&bytes), bytes))
        })
        .collect();
    assert_export_matches_disk(&manager, &id);
    let retry = manager
        .restart(
            &id,
            &directory.path("outputs"),
            demo(),
            "Offline acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap();
    let finished = terminal(&manager, &retry).await;
    assert_eq!(
        (finished.total, finished.saved, finished.failed),
        (cancelled, cancelled, 0)
    );
    assert!(
        manager.snapshot(&id).is_err(),
        "The previous report should not accumulate in memory."
    );
    assert_ne!(finished.output_dir, stopped.output_dir);
    assert!(finished.files.iter().all(|file| stopped
        .files
        .iter()
        .any(|original| original.input == file.input && original.status == "cancelled")));
    assert_sources_unchanged(&completed);
    assert_sources_unchanged(&before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn immediate_demo_cancellation_and_active_job_rejection_are_real_state_transitions() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let input = directory.write(
        "inputs/slow.txt",
        b"Actual product Demo has a cancellable 300 ms wait.",
    );
    let manager = ManagedJob::new();
    let id = start(&manager, &[selected(&input)], &directory.path("outputs"), 1).await;
    assert_eq!(manager.active().as_deref(), Some(id.as_str()));
    let second = manager
        .start(
            request(),
            &[selected(&input)],
            &directory.path("outputs"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(second.contains("already running"));
    let cancelling = manager.cancel(&id).unwrap();
    assert_eq!(cancelling.status, "cancelling");
    let snapshot = terminal(&manager, &id).await;
    assert_eq!(snapshot.status, "stopped");
    assert_eq!((snapshot.total, snapshot.saved, snapshot.failed), (1, 0, 0));
    assert_eq!(snapshot.files[0].status, "cancelled");
    assert!(snapshot.files[0].output.is_none());
    assert!(manager.export_bytes(&id, true).is_err());
    assert_eq!(
        fs::read(input).unwrap(),
        b"Actual product Demo has a cancellable 300 ms wait."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn four_hundred_is_a_global_batch_limit_across_folders_and_zip_entries() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    for index in 0..400 {
        directory.write(
            &format!("boundary/f-{index:03}.txt"),
            b"Boundary file for an actual queued job.",
        );
    }
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("boundary"))],
        &directory.path("accepted"),
        1,
    )
    .await;
    assert_eq!(manager.snapshot(&id).unwrap().total, 400);
    manager.cancel(&id).unwrap();
    let finished = terminal(&manager, &id).await;
    assert_eq!(finished.done, 400);
    directory.write(
        "boundary/one-too-many.txt",
        b"Reject aggregate entry number 401 before provider work.",
    );
    let error = manager
        .start(
            request(),
            &[selected(&directory.path("boundary"))],
            &directory.path("rejected-folder"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("400"));
    assert!(!directory.path("rejected-folder").exists());
    for index in 0..201 {
        directory.write(&format!("first/f-{index:03}.bin"), b"Skipped but counted.");
    }
    for index in 0..200 {
        directory.write(&format!("second/f-{index:03}.bin"), b"Skipped but counted.");
    }
    let error = manager
        .start(
            request(),
            &[
                selected(&directory.path("first")),
                selected(&directory.path("second")),
            ],
            &directory.path("rejected-aggregate"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("400"));
    assert!(!directory.path("rejected-aggregate").exists());
    let numbered_zip = |count| {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..count {
            writer
                .start_file(format!("f-{index:03}.bin"), SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap().into_inner()
    };
    let bytes = numbered_zip(401);
    let zip = directory.write("over-limit.zip", &bytes);
    let error = manager
        .start(
            request(),
            &[selected(&zip)],
            &directory.path("rejected-zip"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("400"));
    assert!(!directory.path("rejected-zip").exists());
    assert_eq!(digest(&fs::read(zip).unwrap()), digest(&bytes));
    let first_zip_bytes = numbered_zip(201);
    let second_zip_bytes = numbered_zip(200);
    let first_zip = directory.write("first.zip", &first_zip_bytes);
    let second_zip = directory.write("second.zip", &second_zip_bytes);
    let error = manager
        .start(
            request(),
            &[selected(&first_zip), selected(&second_zip)],
            &directory.path("rejected-zip-aggregate"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("400"));
    assert!(!directory.path("rejected-zip-aggregate").exists());
    assert_eq!(fs::read(first_zip).unwrap(), first_zip_bytes);
    assert_eq!(fs::read(second_zip).unwrap(), second_zip_bytes);
    assert_eq!(manager.snapshot(&id).unwrap().done, finished.done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn next_unrelated_job_replaces_zip_report_and_rejected_start_keeps_current_report() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let zip_bytes = package(&[("nested/first.txt", b"First archived paragraph.")]);
    let zip_input = directory.write("first.zip", &zip_bytes);
    let second_input = directory.write("second.txt", b"Second unrelated paragraph.");
    let manager = ManagedJob::new();
    let first_id = start(
        &manager,
        &[selected(&zip_input)],
        &directory.path("outputs"),
        1,
    )
    .await;
    let first = terminal(&manager, &first_id).await;
    assert_eq!(first.saved, 1);
    let completed_path = PathBuf::from(first.files[0].output.as_ref().unwrap());
    let completed_bytes = fs::read(&completed_path).unwrap();
    let first_export = manager.export_bytes(&first_id, true).unwrap();
    let mut invalid = request();
    invalid.input_paths = vec![selected(&second_input).path];
    let error = manager
        .start(
            invalid,
            &[selected(&zip_input)],
            &directory.path("rejected"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("selected"));
    assert!(!directory.path("rejected").exists());
    assert_eq!(manager.snapshot(&first_id).unwrap().saved, 1);
    assert_eq!(manager.export_bytes(&first_id, true).unwrap(), first_export);
    let second_id = start(
        &manager,
        &[selected(&second_input)],
        &directory.path("outputs"),
        1,
    )
    .await;
    assert_ne!(first_id, second_id);
    assert!(manager.snapshot(&first_id).is_err());
    assert!(manager.preview(&first_id, None).is_err());
    assert!(manager.export_bytes(&first_id, true).is_err());
    assert!(!manager.owned_output(&completed_path));
    assert_eq!(terminal(&manager, &second_id).await.saved, 1);
    assert_eq!(fs::read(completed_path).unwrap(), completed_bytes);
    assert_eq!(fs::read(zip_input).unwrap(), zip_bytes);
    assert_eq!(
        fs::read(second_input).unwrap(),
        b"Second unrelated paragraph."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_structured_file_exposes_actual_chunk_progress_before_file_completion() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let values: Vec<String> = (0..6)
        .map(|index| format!("Unique chunk {index}: {}", "paragraph text ".repeat(130)))
        .collect();
    let bytes = serde_json::to_vec_pretty(&values).unwrap();
    let source = directory.write("inputs/chunks.json", &bytes);
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&source)],
        &directory.path("outputs"),
        1,
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = manager.snapshot(&id).unwrap();
        if snapshot.chunks_done > 0 && snapshot.chunks_done < snapshot.chunks_total {
            assert_eq!(snapshot.done, 0);
            assert_eq!(snapshot.files[0].status, "running");
            assert_eq!(snapshot.chunks_done, snapshot.files[0].chunks_done);
            assert_eq!(snapshot.chunks_total, snapshot.files[0].chunks_total);
            let serialized = serde_json::to_value(&snapshot).unwrap();
            assert_eq!(serialized["chunksDone"], snapshot.chunks_done);
            assert_eq!(serialized["chunksTotal"], snapshot.chunks_total);
            break;
        }
        assert_eq!(
            snapshot.status, "running",
            "intermediate chunk progress was not observed"
        );
        assert!(
            Instant::now() < deadline,
            "no intermediate chunk progress emitted"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let snapshot = terminal(&manager, &id).await;
    assert_eq!((snapshot.total, snapshot.saved), (1, 1));
    assert!(snapshot.chunks_total > 1);
    assert_eq!(snapshot.chunks_done, snapshot.chunks_total);
    let (_, output) = manager.export_bytes(&id, false).unwrap();
    let translated: Vec<String> = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        translated,
        values
            .iter()
            .map(|value| format!("[zh-Hant] {value}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(digest(&fs::read(source).unwrap()), digest(&bytes));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_file_chunk_totals_survive_partial_cancel_and_reset_on_retry() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    for file in 0..2 {
        let values: Vec<String> = (0..6)
            .map(|chunk| {
                format!(
                    "File {file} chunk {chunk}: {}",
                    "paragraph text ".repeat(130)
                )
            })
            .collect();
        directory.write(
            &format!("inputs/file-{file}.json"),
            &serde_json::to_vec_pretty(&values).unwrap(),
        );
    }
    let originals = snapshot_sources(&directory.path("inputs"));
    let manager = ManagedJob::new();
    let id = start(
        &manager,
        &[selected(&directory.path("inputs"))],
        &directory.path("outputs"),
        2,
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let progress = manager.snapshot(&id).unwrap();
        assert_eq!(
            progress.chunks_done,
            progress.files.iter().map(|f| f.chunks_done).sum::<usize>()
        );
        assert_eq!(
            progress.chunks_total,
            progress.files.iter().map(|f| f.chunks_total).sum::<usize>()
        );
        if progress.files.iter().all(|f| f.chunks_total > 1)
            && progress.chunks_done > 0
            && progress.chunks_done < progress.chunks_total
        {
            manager.cancel(&id).unwrap();
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no actual concurrent partial progress observed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let stopped = terminal(&manager, &id).await;
    assert_eq!(stopped.status, "stopped");
    assert_eq!(stopped.failed, 0);
    let cancelled = stopped
        .files
        .iter()
        .filter(|f| f.status == "cancelled")
        .count();
    assert!(cancelled > 0);
    assert_eq!(stopped.saved + cancelled, 2);
    assert!(stopped.chunks_done < stopped.chunks_total);
    let completed: BTreeMap<PathBuf, Vec<u8>> = stopped
        .files
        .iter()
        .filter_map(|f| {
            f.output
                .as_ref()
                .map(|p| (PathBuf::from(p), fs::read(p).unwrap()))
        })
        .collect();
    let retry = manager
        .restart(
            &id,
            &directory.path("outputs"),
            demo(),
            "Acceptance".into(),
            2,
            1,
            None,
        )
        .await
        .unwrap();
    let initial = manager.snapshot(&retry).unwrap();
    assert_eq!(initial.total, cancelled);
    assert_eq!(initial.chunks_done, 0);
    assert!(initial.files.iter().all(|f| f.chunks_done == 0));
    let finished = terminal(&manager, &retry).await;
    assert_eq!(finished.saved, cancelled);
    assert_eq!(finished.chunks_done, finished.chunks_total);
    assert_eq!(
        finished.chunks_done,
        finished.files.iter().map(|f| f.chunks_done).sum::<usize>()
    );
    for (path, bytes) in completed {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_sources_unchanged(&originals);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_size_limit_is_checked_before_provider_or_output_work() {
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    let input = directory.path("oversized.txt");
    let file = fs::File::create(&input).unwrap();
    file.set_len(80_000_001).unwrap();
    drop(file);
    let manager = ManagedJob::new();
    let error = manager
        .start(
            request(),
            &[selected(&input)],
            &directory.path("rejected"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("byte limit"));
    assert!(manager.active().is_none());
    assert!(!directory.path("rejected").exists());
    assert_eq!(fs::metadata(&input).unwrap().len(), 80_000_001);
    let bounded = directory.write("bounded.txt", b"123456789");
    assert!(jobs::read_bounded(&bounded, 8).is_err());
    assert_eq!(jobs::read_bounded(&bounded, 9).unwrap(), b"123456789");
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_windows_folder_junction_loop_is_skipped_and_direct_selection_rejected() {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::process::CommandExt;
    let _demo = DemoEnvironment::new(true);
    let directory = TestDirectory::new();
    directory.write("inputs/manual.txt", b"Real junction regression input.");
    let root = directory.path("inputs").canonicalize().unwrap();
    let junction = root.join("loop");
    // mklink /J creates an NTFS junction without administrator privileges. These generated
    // path arguments are validated before invoking the Windows builtin, and no deletion uses cmd.
    for path in [&junction, &root] {
        assert!(!path
            .to_string_lossy()
            .chars()
            .any(|c| "&|<>^%!\"\r\n".contains(c)));
    }
    let result = std::process::Command::new("cmd.exe")
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "NTFS junction creation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = fs::symlink_metadata(&junction).unwrap();
    assert_ne!(metadata.file_attributes() & 0x400, 0);
    assert!(jobs::is_reparse(&metadata));
    let manager = ManagedJob::new();
    let direct = SelectedInput {
        path: junction.to_string_lossy().into(),
        name: "loop".into(),
        size: 0,
        kind: "folder".into(),
    };
    let error = manager
        .start(
            request(),
            &[direct],
            &directory.path("direct-rejected"),
            demo(),
            "Acceptance".into(),
            1,
            1,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("junction"));
    assert!(!directory.path("direct-rejected").exists());
    let id = start(&manager, &[selected(&root)], &directory.path("accepted"), 1).await;
    let snapshot = terminal(&manager, &id).await;
    assert_eq!((snapshot.total, snapshot.saved, snapshot.failed), (1, 1, 0));
    assert_eq!(snapshot.files[0].relative, "manual.txt");
    fs::remove_dir(&junction).unwrap();
    assert!(root.exists());
    assert_eq!(
        fs::read(root.join("manual.txt")).unwrap(),
        b"Real junction regression input."
    );
}
