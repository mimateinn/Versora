//! Native job coordination: registered inputs, bounded ZIP reads and new atomic output files.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use versora_core::names::{english_job_name, output_name};
use versora_engine::{
    formats,
    providers::{CancelToken, ProviderConfig, ProviderRuntime},
    translate::TranslationOptions,
};

const MAX_BYTES: u64 = 80_000_000;
const MAX_ARCHIVE_BYTES: u64 = MAX_BYTES + 4_000_000;
const MAX_FILES: usize = 400;
static COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize)]
pub struct SelectedInput {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    #[serde(default)]
    pub input_paths: Vec<String>,
    #[serde(default)]
    pub source_language: Option<String>,
    pub target_language: String,
    #[serde(default = "general")]
    pub purpose_id: String,
    #[serde(default = "auto")]
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default = "document")]
    pub mode: String,
}
fn general() -> String {
    "general".into()
}
fn auto() -> String {
    "auto".into()
}
fn document() -> String {
    "document".into()
}

#[derive(Clone, Serialize)]
pub struct JobFile {
    pub input: String,
    pub name: String,
    pub relative: String,
    pub status: String,
    pub chunks_done: usize,
    pub chunks_total: usize,
    pub used_demo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub status: String,
    pub total: usize,
    pub done: usize,
    pub saved: usize,
    pub failed: usize,
    pub skipped: usize,
    pub current: String,
    pub chunks_done: usize,
    pub chunks_total: usize,
    pub used_demo: bool,
    pub message: String,
    pub output_dir: String,
    pub files: Vec<JobFile>,
}
#[derive(Clone)]
struct Input {
    original: String,
    relative: PathBuf,
    disk: Option<PathBuf>,
    bytes: Option<Arc<Vec<u8>>>,
    skipped: Option<String>,
}
struct Job {
    snapshot: JobSnapshot,
    cancel: CancelToken,
    request: TranslationRequest,
    inputs: Vec<Input>,
}

#[derive(Default)]
pub struct JobManager {
    jobs: Mutex<HashMap<String, Job>>,
    active: Mutex<Option<String>>,
}
impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn active(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }
    pub fn snapshot(&self, id: &str) -> Result<JobSnapshot, String> {
        self.jobs
            .lock()
            .unwrap()
            .get(id)
            .map(|j| j.snapshot.clone())
            .ok_or_else(|| "Unknown job.".into())
    }
    pub fn cancel(&self, id: &str) -> Result<JobSnapshot, String> {
        let mut jobs = self.jobs.lock().unwrap();
        let job = jobs.get_mut(id).ok_or("Unknown job.")?;
        if matches!(job.snapshot.status.as_str(), "running" | "cancelling") {
            job.cancel.store(true, Ordering::SeqCst);
            job.snapshot.status = "cancelling".into();
            job.snapshot.message = "Stopping active work; completed output is kept.".into();
        }
        Ok(job.snapshot.clone())
    }
    fn retry(&self, id: &str) -> Result<(TranslationRequest, Vec<Input>), String> {
        let jobs = self.jobs.lock().unwrap();
        let job = jobs.get(id).ok_or("Unknown job.")?;
        if matches!(job.snapshot.status.as_str(), "running" | "cancelling") {
            return Err("Wait for the running job to stop.".into());
        }
        let inputs = job
            .inputs
            .iter()
            .zip(&job.snapshot.files)
            .filter(|(_, f)| matches!(f.status.as_str(), "failed" | "cancelled" | "pending"))
            .map(|(i, _)| i.clone())
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            return Err("No unfinished files to retry.".into());
        }
        Ok((job.request.clone(), inputs))
    }
    pub fn retry_request(&self, id: &str) -> Result<TranslationRequest, String> {
        self.retry(id).map(|(request, _)| request)
    }
    pub fn preview(&self, id: &str, input: Option<&str>) -> Result<serde_json::Value, String> {
        let (source, output) = {
            let jobs = self.jobs.lock().unwrap();
            let job = jobs.get(id).ok_or("Unknown job.")?;
            let index = job
                .inputs
                .iter()
                .position(|i| input.is_none_or(|p| p == i.original))
                .ok_or("Input does not belong to this job.")?;
            (
                job.inputs[index].clone(),
                job.snapshot.files[index].output.clone(),
            )
        };
        let source = match source.bytes {
            Some(bytes) => bytes.as_ref().clone(),
            None => read_bounded(
                source.disk.as_ref().ok_or("No preview for this entry.")?,
                MAX_BYTES,
            )?,
        };
        let translated = output
            .map(|path| read_bounded(Path::new(&path), MAX_BYTES))
            .transpose()?;
        Ok(super::preview_text(&source, translated.as_deref()))
    }
    pub fn owned_output(&self, path: &Path) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        self.jobs.lock().unwrap().values().any(|j| {
            let Ok(root) = Path::new(&j.snapshot.output_dir).canonicalize() else {
                return false;
            };
            path.starts_with(root)
        })
    }
    pub fn export_bytes(&self, id: &str, zip_output: bool) -> Result<(String, Vec<u8>), String> {
        let snap = self.snapshot(id)?;
        let saved = snap
            .files
            .iter()
            .filter(|f| f.status == "saved")
            .collect::<Vec<_>>();
        if saved.is_empty() {
            return Err("No completed output to export.".into());
        }
        if !zip_output && saved.len() == 1 {
            let path = PathBuf::from(saved[0].output.as_ref().ok_or("Missing saved output.")?);
            return Ok((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                read_export_file(&path, Path::new(&snap.output_dir), MAX_BYTES)?,
            ));
        }
        let mut buffer = BoundedArchive::new(MAX_ARCHIVE_BYTES);
        let mut remaining = MAX_BYTES;
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            for file in saved {
                let path = PathBuf::from(file.output.as_ref().ok_or("Missing saved output.")?);
                let relative = path
                    .strip_prefix(&snap.output_dir)
                    .map_err(|_| "Output outside job root.")?;
                let bytes = read_export_file(&path, Path::new(&snap.output_dir), remaining)?;
                remaining -= bytes.len() as u64;
                writer
                    .start_file(
                        relative.to_string_lossy().replace('\\', "/"),
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .map_err(|e| e.to_string())?;
                writer.write_all(&bytes).map_err(|e| e.to_string())?;
            }
            writer.finish().map_err(|e| e.to_string())?;
        }
        Ok((
            format!("Versora-{}.zip", snap.id),
            buffer.inner.into_inner(),
        ))
    }
    fn mutate(&self, id: &str, app: Option<&AppHandle>, update: impl FnOnce(&mut JobSnapshot)) {
        let snap = {
            let mut jobs = self.jobs.lock().unwrap();
            if let Some(j) = jobs.get_mut(id) {
                update(&mut j.snapshot);
                recount(&mut j.snapshot);
                Some(j.snapshot.clone())
            } else {
                None
            }
        };
        if let (Some(app), Some(snap)) = (app, snap) {
            let _ = app.emit("job-progress", snap);
        }
    }
    pub async fn start(
        self: &Arc<Self>,
        request: TranslationRequest,
        selected: &[SelectedInput],
        output_parent: &Path,
        configs: Vec<ProviderConfig>,
        system: String,
        concurrency: usize,
        per_provider: usize,
        app: Option<AppHandle>,
    ) -> Result<String, String> {
        let inputs = collect_inputs(selected, &request)?;
        self.start_inputs(
            request,
            inputs,
            output_parent,
            configs,
            system,
            concurrency,
            per_provider,
            app,
        )
        .await
    }
    pub async fn restart(
        self: &Arc<Self>,
        previous: &str,
        output_parent: &Path,
        configs: Vec<ProviderConfig>,
        system: String,
        concurrency: usize,
        per_provider: usize,
        app: Option<AppHandle>,
    ) -> Result<String, String> {
        let (request, inputs) = self.retry(previous)?;
        self.start_inputs(
            request,
            inputs,
            output_parent,
            configs,
            system,
            concurrency,
            per_provider,
            app,
        )
        .await
    }
    async fn start_inputs(
        self: &Arc<Self>,
        request: TranslationRequest,
        inputs: Vec<Input>,
        output_parent: &Path,
        configs: Vec<ProviderConfig>,
        system: String,
        concurrency: usize,
        per_provider: usize,
        app: Option<AppHandle>,
    ) -> Result<String, String> {
        if request.target_language.trim().is_empty() || request.target_language.chars().count() > 80
        {
            return Err("Choose a target language (up to 80 characters).".into());
        }
        if inputs.is_empty() {
            return Err("Choose a supported input file.".into());
        }
        if configs.is_empty() {
            return Err("No usable translator. Configure a provider in Settings.".into());
        }
        let id = new_id();
        let mut active = self.active.lock().unwrap();
        if active.is_some() {
            return Err("A translation job is already running.".into());
        }
        fs::create_dir_all(output_parent).map_err(|e| e.to_string())?;
        let tag = english_job_name("translation", &inputs[0].relative.to_string_lossy());
        let root = output_parent.join(format!("{tag}-{id}"));
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        // Capture the original resolved directory, not a later replacement junction.
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        let files = inputs
            .iter()
            .map(|i| JobFile {
                input: i.original.clone(),
                name: i
                    .relative
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                relative: i.relative.to_string_lossy().replace('\\', "/"),
                status: if i.skipped.is_some() {
                    "skipped"
                } else {
                    "pending"
                }
                .into(),
                chunks_done: 0,
                chunks_total: 0,
                used_demo: false,
                output: None,
                error: i.skipped.clone(),
            })
            .collect();
        let mut snap = JobSnapshot {
            id: id.clone(),
            status: "running".into(),
            total: inputs.len(),
            done: 0,
            saved: 0,
            failed: 0,
            skipped: 0,
            current: String::new(),
            chunks_done: 0,
            chunks_total: 0,
            used_demo: false,
            message: String::new(),
            output_dir: root.to_string_lossy().into(),
            files,
        };
        recount(&mut snap);
        // The UI exposes the current report only. Drop old ZIP bytes/reports on a new job;
        // their completed output files remain untouched on disk.
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.clear();
            jobs.insert(
                id.clone(),
                Job {
                    snapshot: snap,
                    cancel: cancel.clone(),
                    request: request.clone(),
                    inputs: inputs.clone(),
                },
            );
        }
        *active = Some(id.clone());
        drop(active);
        let manager = self.clone();
        let task_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let runtime = Arc::new(ProviderRuntime::new(
                concurrency.clamp(1, 16),
                per_provider.clamp(1, 8),
            ));
            let gates = Arc::new(Semaphore::new(concurrency.clamp(1, 16)));
            let mut tasks = tokio::task::JoinSet::new();
            for (index, input) in inputs.into_iter().enumerate() {
                if input.skipped.is_some() {
                    continue;
                }
                let manager = manager.clone();
                let id = task_id.clone();
                let runtime = Arc::new(runtime.for_operation());
                let gates = gates.clone();
                let configs = configs.clone();
                let system = system.clone();
                let cancel = cancel.clone();
                let request = request.clone();
                let root = root.clone();
                let app = app.clone();
                tasks.spawn(async move {
                    let Ok(_permit) = gates.acquire_owned().await else {
                        return;
                    };
                    if cancel.load(Ordering::SeqCst) {
                        manager.mutate(&id, app.as_ref(), |s| {
                            s.files[index].status = "cancelled".into()
                        });
                        return;
                    }
                    manager.mutate(&id, app.as_ref(), |s| {
                        s.files[index].status = "running".into();
                        s.current = s.files[index].relative.clone();
                    });
                    let ext = input
                        .relative
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    let mode = request.mode.clone();
                    let parsed = tokio::task::spawn_blocking(move || {
                        let bytes = match input.bytes {
                            Some(bytes) => bytes.as_ref().clone(),
                            None => read_bounded(
                                input.disk.as_ref().ok_or("Missing source file.")?,
                                MAX_BYTES,
                            )?,
                        };
                        formats::extract(&bytes, &ext, &mode).map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| v);
                    let outcome = match parsed {
                        Err(error) => Err(error),
                        Ok(doc) => {
                            let options = TranslationOptions {
                                target_lang: request.target_language.clone(),
                                max_chars: 3000,
                                global_limit: concurrency.clamp(1, 16),
                                per_provider_limit: per_provider.clamp(1, 8),
                            };
                            let progress_manager = manager.clone();
                            let progress_id = id.clone();
                            let progress_app = app.clone();
                            let progress: versora_engine::translate::Progress =
                                Arc::new(move |done, total| {
                                    progress_manager.mutate(
                                        &progress_id,
                                        progress_app.as_ref(),
                                        |s| {
                                            s.files[index].chunks_done = done;
                                            s.files[index].chunks_total = total;
                                        },
                                    )
                                });
                            let result = runtime
                                .translate_texts(
                                    &doc.units,
                                    &options,
                                    &configs,
                                    &system,
                                    &cancel,
                                    Some(progress),
                                )
                                .await
                                .map_err(|e| e.to_string());
                            match result {
                                Err(error) => Err(error),
                                Ok(translations) => {
                                    if cancel.load(Ordering::SeqCst) {
                                        Err("Translation cancelled.".into())
                                    } else {
                                        let bytes = formats::write_document(&doc, &translations)
                                            .map_err(|e| e.to_string());
                                        match bytes {
                                            Err(error) => Err(error),
                                            Ok(bytes) => {
                                                let relative = PathBuf::from(
                                                    manager.snapshot(&id).unwrap().files[index]
                                                        .relative
                                                        .clone(),
                                                );
                                                let base =
                                                    relative.file_name().unwrap().to_string_lossy();
                                                let name =
                                                    output_name(&base, &request.target_language)
                                                        .map_err(|e| e.to_string());
                                                match name {
                                                    Err(error) => Err(error),
                                                    Ok(name) => {
                                                        let folder = root.join(
                                                            relative
                                                                .parent()
                                                                .unwrap_or(Path::new("")),
                                                        );
                                                        fs::create_dir_all(&folder)
                                                            .map_err(|e| e.to_string())
                                                            .and_then(|_| {
                                                                atomic_output(
                                                                    &folder, &name, &bytes,
                                                                )
                                                            })
                                                            .map(|p| {
                                                                p.to_string_lossy().into_owned()
                                                            })
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    };
                    manager.mutate(&id, app.as_ref(), |s| match outcome {
                        Ok(path) => {
                            s.files[index].status = "saved".into();
                            s.files[index].output = Some(path);
                            s.files[index].used_demo = runtime.used_demo();
                        }
                        Err(error) => {
                            s.files[index].status = if cancel.load(Ordering::SeqCst) {
                                "cancelled"
                            } else {
                                "failed"
                            }
                            .into();
                            s.files[index].error = Some(error);
                        }
                    });
                });
            }
            while let Some(result) = tasks.join_next().await {
                if let Err(error) = result {
                    manager.mutate(&task_id, app.as_ref(), |s| {
                        s.message = format!("Worker stopped: {error}")
                    });
                }
            }
            manager.mutate(&task_id, app.as_ref(), |s| {
                for file in &mut s.files {
                    if matches!(file.status.as_str(), "pending" | "running") {
                        file.status = "failed".into();
                        file.error = Some("Worker did not complete this file.".into());
                    }
                }
                s.current.clear();
                s.status = if cancel.load(Ordering::SeqCst) {
                    "stopped"
                } else if s.saved == 0 && s.failed > 0 {
                    "error"
                } else {
                    "done"
                }
                .into();
            });
            *manager.active.lock().unwrap() = None;
        });
        Ok(id)
    }
}

fn recount(s: &mut JobSnapshot) {
    s.used_demo = s.files.iter().any(|f| f.status == "saved" && f.used_demo);
    s.chunks_done = s.files.iter().map(|f| f.chunks_done).sum();
    s.chunks_total = s.files.iter().map(|f| f.chunks_total).sum();
    s.saved = s.files.iter().filter(|f| f.status == "saved").count();
    s.failed = s.files.iter().filter(|f| f.status == "failed").count();
    s.skipped = s.files.iter().filter(|f| f.status == "skipped").count();
    s.done = s
        .files
        .iter()
        .filter(|f| !matches!(f.status.as_str(), "pending" | "running"))
        .count();
}
fn new_id() -> String {
    format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
struct BoundedArchive {
    inner: std::io::Cursor<Vec<u8>>,
    limit: u64,
}
impl BoundedArchive {
    fn new(limit: u64) -> Self {
        Self {
            inner: std::io::Cursor::new(Vec::new()),
            limit,
        }
    }
}
impl Write for BoundedArchive {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .inner
            .position()
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.limit)
        {
            return Err(std::io::Error::other(
                "Export archive exceeds the 84 MB limit.",
            ));
        }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for BoundedArchive {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let old = self.inner.position();
        let next = self.inner.seek(position)?;
        if next > self.limit {
            self.inner.set_position(old);
            return Err(std::io::Error::other(
                "Export archive exceeds the 84 MB limit.",
            ));
        }
        Ok(next)
    }
}
fn read_export_file(path: &Path, root: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Output outside job root.")?;
    if !safe_relative(relative) {
        return Err("Unsafe output path.".into());
    }
    let mut component_path = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            component_path.push(component.as_os_str());
        }
        let metadata = fs::symlink_metadata(&component_path).map_err(|e| e.to_string())?;
        if is_reparse(&metadata) {
            return Err("Replacement output links or junctions are not exported.".into());
        }
    }
    read_bounded(path, limit)
}
#[cfg(windows)]
fn opened_path(file: &File) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let handle = file.as_raw_handle();
    let required = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0) };
    if required == 0 || required > 32768 {
        return Err("Cannot verify the opened file path.".into());
    }
    let mut buffer = vec![0u16; required as usize + 1];
    let count =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
    if count == 0 || count as usize >= buffer.len() {
        return Err("Cannot verify the opened file path.".into());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..count as usize],
    )))
}
#[cfg(windows)]
fn same_windows_path(left: &Path, right: &Path) -> bool {
    fn familiar(path: &Path) -> String {
        let text = path.to_string_lossy();
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{unc}")
        } else {
            text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned()
        }
    }
    familiar(left).eq_ignore_ascii_case(&familiar(right))
}
pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if is_reparse(&meta) || !meta.is_file() || meta.len() > limit {
        return Err("File exceeds limit or is not a regular file.".into());
    }
    #[cfg(windows)]
    if !same_windows_path(&opened_path(&file)?, path) {
        return Err("The opened file no longer matches the registered path.".into());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("File grew beyond the size limit.".into());
    }
    Ok(bytes)
}
pub(crate) fn is_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.components().all(|c| {
            let Component::Normal(part) = c else {
                return false;
            };
            let s = part.to_string_lossy();
            !s.ends_with(['.', ' '])
                && !s
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
                && !versora_core::glossary::is_windows_device(&s)
        })
}
fn skipped_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "venv" | "__pycache__" | "node_modules")
}
fn collect_inputs(
    selected: &[SelectedInput],
    request: &TranslationRequest,
) -> Result<Vec<Input>, String> {
    let selected = selected
        .iter()
        .filter(|s| request.input_paths.is_empty() || request.input_paths.contains(&s.path))
        .collect::<Vec<_>>();
    if !request.input_paths.is_empty()
        && request
            .input_paths
            .iter()
            .any(|p| !selected.iter().any(|s| s.path == *p))
    {
        return Err("Input was not selected through the native picker.".into());
    }
    let mut inputs = Vec::new();
    let mut total = 0u64;
    for selected in selected {
        let path = PathBuf::from(&selected.path);
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_reparse(&meta) {
            return Err("Symbolic-link or junction inputs are not followed.".into());
        }
        if meta.is_dir() {
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            walk(
                &canonical,
                &canonical,
                &request.mode,
                &mut inputs,
                &mut total,
            )?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        {
            let bytes = read_bounded(&path, MAX_BYTES)?;
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if archive.len() > MAX_FILES {
                return Err(format!("ZIP exceeds {MAX_FILES} entries."));
            }
            for index in 0..archive.len() {
                let file = archive.by_index(index).map_err(|e| e.to_string())?;
                total = total.checked_add(file.size()).ok_or("ZIP size overflow.")?;
                if total > MAX_BYTES {
                    return Err(format!("ZIP expanded contents exceed {MAX_BYTES} bytes."));
                }
            }
            for index in 0..archive.len() {
                let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
                if file.is_dir() {
                    continue;
                }
                let original = file.name().to_owned();
                let relative = PathBuf::from(original.replace('\\', "/"));
                let unsafe_path = !safe_relative(&relative)
                    || file.enclosed_name().is_none()
                    || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
                let supported = formats::supported_extension(
                    &relative.extension().unwrap_or_default().to_string_lossy(),
                    &request.mode,
                );
                let skip = if unsafe_path {
                    Some("Unsafe ZIP entry path.".into())
                } else if !supported {
                    Some("Unsupported file type.".into())
                } else {
                    None
                };
                let data = if skip.is_none() {
                    let mut data = Vec::new();
                    file.by_ref()
                        .take(MAX_BYTES + 1)
                        .read_to_end(&mut data)
                        .map_err(|e| e.to_string())?;
                    if data.len() as u64 > MAX_BYTES {
                        return Err("ZIP entry exceeds size limit.".into());
                    }
                    Some(Arc::new(data))
                } else {
                    None
                };
                inputs.push(Input {
                    original: format!("{}::{original}", path.display()),
                    relative,
                    disk: None,
                    bytes: data,
                    skipped: skip,
                });
            }
        } else {
            total = total.checked_add(meta.len()).ok_or("Size overflow.")?;
            if total > MAX_BYTES {
                return Err("Selected input exceeds batch byte limit.".into());
            }
            let relative = PathBuf::from(path.file_name().ok_or("Input needs a filename.")?);
            let ext = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            inputs.push(Input {
                original: path.to_string_lossy().into(),
                relative,
                disk: Some(path),
                bytes: None,
                skipped: if formats::supported_extension(&ext, &request.mode) {
                    None
                } else {
                    Some("Unsupported file type.".into())
                },
            });
        }
        if inputs.len() > MAX_FILES {
            return Err(format!("Batch exceeds {MAX_FILES} files."));
        }
    }
    Ok(inputs)
}
fn walk(
    root: &Path,
    current: &Path,
    mode: &str,
    inputs: &mut Vec<Input>,
    total: &mut u64,
) -> Result<(), String> {
    let mut children = fs::read_dir(current)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_reparse(&meta) {
            continue;
        }
        if meta.is_dir() {
            if !skipped_dir(&entry.file_name().to_string_lossy()) {
                let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                if canonical.starts_with(root) {
                    walk(root, &canonical, mode, inputs, total)?;
                }
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        *total = total.checked_add(meta.len()).ok_or("Size overflow.")?;
        if *total > MAX_BYTES {
            return Err("Folder exceeds batch byte limit.".into());
        }
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(root) {
            continue;
        }
        let relative = canonical
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_path_buf();
        let ext = relative
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        inputs.push(Input {
            original: canonical.to_string_lossy().into(),
            relative,
            disk: Some(canonical),
            bytes: None,
            skipped: if formats::supported_extension(&ext, mode) {
                None
            } else {
                Some("Unsupported file type.".into())
            },
        });
        if inputs.len() > MAX_FILES {
            return Err(format!("Folder exceeds {MAX_FILES} files."));
        }
    }
    Ok(())
}

pub fn atomic_output(folder: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if !safe_relative(Path::new(name)) || Path::new(name).components().count() != 1 {
        return Err("Unsafe output name.".into());
    }
    let temp = folder.join(format!(".versora-{}.tmp", new_id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(error.to_string());
    }
    drop(file);
    let mut counter = 1;
    loop {
        let name = if counter == 1 {
            name.to_owned()
        } else {
            let p = Path::new(name);
            format!(
                "{} ({counter}){}",
                p.file_stem().unwrap().to_string_lossy(),
                p.extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default()
            )
        };
        let target = folder.join(name);
        #[cfg(windows)]
        let result = {
            use std::os::windows::ffi::OsStrExt;
            let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let destination: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    source.as_ptr(),
                    destination.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
                )
            } != 0
            {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        };
        #[cfg(not(windows))]
        let result = fs::hard_link(&temp, &target).and_then(|_| fs::remove_file(&temp));
        match result {
            Ok(()) => return Ok(target),
            Err(_error) if target.exists() => {
                counter += 1;
                if counter > 10000 {
                    let _ = fs::remove_file(&temp);
                    return Err("Too many output collisions.".into());
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zip_paths_are_closed() {
        for path in [
            "../escape.txt",
            "C:/secret.txt",
            "/root.txt",
            "a/../b.txt",
            "NUL.txt",
            "a/stream:secret.txt",
            "a/trim. ",
        ] {
            assert!(!safe_relative(Path::new(path)), "{path}");
        }
        assert!(safe_relative(Path::new("章節/安全.txt")));
    }
    #[test]
    fn output_is_new_and_collision_keeps_old() {
        let root = std::env::temp_dir().join(format!("versora-atomic-{}", new_id()));
        fs::create_dir(&root).unwrap();
        let a = atomic_output(&root, "test.txt", b"original").unwrap();
        let b = atomic_output(&root, "test.txt", b"new").unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(a).unwrap(), b"original");
        assert_eq!(fs::read(b).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn zip_cap_preflights_all_entries() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut bytes);
            z.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"evil").unwrap();
            z.finish().unwrap();
        }
        let root = std::env::temp_dir().join(format!("versora-zip-{}", new_id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("source.zip");
        fs::write(&path, bytes.into_inner()).unwrap();
        let selected = [SelectedInput {
            path: path.to_string_lossy().into(),
            name: "source.zip".into(),
            size: 0,
            kind: "zip".into(),
        }];
        let request = TranslationRequest {
            target_language: "zh-Hant".into(),
            mode: "document".into(),
            ..Default::default()
        };
        let inputs = collect_inputs(&selected, &request).unwrap();
        assert!(inputs[0].skipped.as_ref().unwrap().contains("Unsafe"));
        assert!(!root.join("evil.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
