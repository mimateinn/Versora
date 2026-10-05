//! Shell-free local transports. Windows children are suspended until their kill-on-close job owns them.
use crate::providers::{
    check_cancel, is_secret_name, redact, CancelToken, ProviderConfig, ProviderError,
    ProviderStatus, CALL_TIMEOUT, OUTPUT_CAP,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct CliResult {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub done_early: bool,
}
#[derive(Clone)]
pub struct RunOptions {
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub stdin_text: Option<String>,
    pub timeout: Duration,
    pub first_byte: Option<Duration>,
    pub idle: Option<Duration>,
    pub grok_terminal: bool,
}
impl RunOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            env: child_env(),
            stdin_text: None,
            timeout: CALL_TIMEOUT,
            first_byte: None,
            idle: None,
            grok_terminal: false,
        }
    }
}
pub fn child_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(k, _)| !is_secret_name(&k.to_string_lossy()))
        .collect()
}

#[cfg(windows)]
mod windows_job {
    use std::{
        ffi::c_void,
        io, mem,
        os::windows::{io::AsRawHandle, process::CommandExt},
        process::{Child, Command},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{
                OpenThread, ResumeThread, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
                CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
            },
        },
    };
    pub struct Job(HANDLE);
    unsafe impl Send for Job {}
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn configure(command: &mut Command) {
        command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    pub fn attach_resume(child: &Child) -> io::Result<Job> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Job(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = mem::zeroed();
            entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
            let mut found = false;
            let mut ok = Thread32First(snapshot, &mut entry);
            while ok != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        let result = ResumeThread(thread);
                        CloseHandle(thread);
                        if result != u32::MAX {
                            found = true;
                        }
                    }
                }
                ok = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if !found {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Could not resume owned CLI primary thread",
                ));
            }
            Ok(job)
        }
    }
}
struct Capture {
    out: Vec<u8>,
    err: Vec<u8>,
    over: bool,
    last: Option<Instant>,
}
fn pump(
    mut stream: impl Read + Send + 'static,
    state: Arc<Mutex<Capture>>,
    stdout: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
                    if state.out.len() + state.err.len() + n > OUTPUT_CAP {
                        state.over = true;
                        return;
                    }
                    if stdout {
                        state.out.extend_from_slice(&buf[..n]);
                        state.last = Some(Instant::now());
                    } else {
                        state.err.extend_from_slice(&buf[..n]);
                    }
                }
            }
        }
    })
}
/// Run a concrete executable and argv. The caller owns timeout/cancellation; no shell expansion.
pub async fn run_cli(
    binary: &Path,
    args: &[String],
    options: RunOptions,
    cancel: &CancelToken,
) -> Result<CliResult, ProviderError> {
    check_cancel(cancel)?;
    let mut command = Command::new(binary);
    command
        .args(args)
        .current_dir(&options.cwd)
        .env_clear()
        .envs(
            options
                .env
                .into_iter()
                .filter(|(name, _)| !is_secret_name(&name.to_string_lossy())),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if options.stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    #[cfg(windows)]
    windows_job::configure(&mut command);
    let mut child = command.spawn().map_err(|e| {
        ProviderError::new("spawn", format!("CLI could not start: {}", e.kind()), "")
    })?;
    #[cfg(windows)]
    let job = match windows_job::attach_resume(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::new(
                "spawn",
                format!("Windows CLI process containment failed: {}", e.kind()),
                "",
            ));
        }
    };
    let state = Arc::new(Mutex::new(Capture {
        out: Vec::new(),
        err: Vec::new(),
        over: false,
        last: None,
    }));
    let out = pump(
        child.stdout.take().expect("piped stdout"),
        state.clone(),
        true,
    );
    let err = pump(
        child.stderr.take().expect("piped stderr"),
        state.clone(),
        false,
    );
    let feed = options.stdin_text.map(|text| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        })
    });
    let start = Instant::now();
    let mut reason = "";
    let code = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| ProviderError::new("spawn", "CLI process status failed", ""))?
        {
            break status.code();
        }
        {
            let capture = state.lock().unwrap_or_else(|p| p.into_inner());
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                reason = "cancel";
            } else if capture.over {
                reason = "over";
            } else if start.elapsed() > options.timeout
                || options
                    .first_byte
                    .is_some_and(|t| capture.last.is_none() && start.elapsed() > t)
                || options
                    .idle
                    .is_some_and(|t| capture.last.is_some_and(|last| last.elapsed() > t))
            {
                reason = "timeout";
            } else if options.grok_terminal && grok_finished(&capture.out) {
                reason = "done";
            }
        }
        if !reason.is_empty() {
            let _ = child.kill();
            break child.wait().ok().and_then(|s| s.code());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    // Close the job before waiting on pipes: a descendant may have inherited them.
    #[cfg(windows)]
    drop(job);
    out.join()
        .map_err(|_| ProviderError::new("spawn", "CLI stdout reader failed", ""))?;
    err.join()
        .map_err(|_| ProviderError::new("spawn", "CLI stderr reader failed", ""))?;
    if let Some(feed) = feed {
        let _ = feed.join();
    }
    let capture = state.lock().unwrap_or_else(|p| p.into_inner());
    if reason == "cancel" {
        return Err(ProviderError::cancelled());
    }
    if reason == "over" || capture.over {
        return Err(ProviderError::new(
            "truncated",
            "CLI stdout and stderr exceeded 4 MiB",
            "",
        ));
    }
    Ok(CliResult {
        code,
        stdout: String::from_utf8_lossy(&capture.out).into_owned(),
        stderr: String::from_utf8_lossy(&capture.err).into_owned(),
        timed_out: reason == "timeout",
        done_early: reason == "done",
    })
}
fn safe_arg(value: &str) -> bool {
    value.is_empty()
        || value.len() <= 64
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}
fn cli_name(id: &str) -> Option<&'static str> {
    match id {
        "claude_cli" => Some("claude"),
        "codex_cli" => Some("codex"),
        "grok_cli" => Some("grok"),
        _ => None,
    }
}
fn native_candidate(path: &Path, name: &str) -> Option<PathBuf> {
    if path.is_file()
        && path
            .file_stem()
            .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name))
        && (cfg!(not(windows))
            || path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("exe")))
    {
        return path.canonicalize().ok();
    }
    None
}
/// Never execute .cmd/.bat wrappers. Known official npm Codex layout can resolve its native exe.
pub fn resolve_bin(config: &ProviderConfig) -> Option<PathBuf> {
    let name = cli_name(&config.id)?;
    if !config.cli_path.trim().is_empty() {
        return native_candidate(Path::new(config.cli_path.trim()), name);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if let Some(path) = native_candidate(
            &dir.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            }),
            name,
        ) {
            return Some(path);
        }
    }
    if cfg!(windows) && name == "codex" {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            let base = PathBuf::from(roaming).join("npm/node_modules/@openai/codex");
            for path in [base.join("node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe"),base.join("vendor/x86_64-pc-windows-msvc/codex/codex.exe")] {if let Some(path)=native_candidate(&path,name){return Some(path);}}
        }
    }
    None
}
pub fn preset_args(
    id: &str,
    model: &str,
    effort: &str,
    prompt: &Path,
    cwd: &Path,
) -> Result<Vec<String>, ProviderError> {
    if !safe_arg(model) || !safe_arg(effort) {
        return Err(ProviderError::new(
            "spawn",
            "Invalid model or effort identifier",
            id,
        ));
    }
    let strings = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut args = match id {
        "claude_cli" => strings(&[
            "-p",
            "--tools",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
        ]),
        "codex_cli" => {
            let mut a = strings(&[
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "--cd",
            ]);
            a.push(cwd.to_string_lossy().into_owned());
            a
        }
        "grok_cli" => {
            let mut a = strings(&["--prompt-file"]);
            a.push(prompt.to_string_lossy().into_owned());
            a.extend(strings(&[
                "--output-format",
                "streaming-json",
                "--tools=Read",
                "--deny",
                "Read",
                "--deny",
                "MCPTool",
                "--permission-mode",
                "dontAsk",
                "--no-plan",
                "--no-subagents",
                "--disable-web-search",
                "--max-turns",
                "1",
                "--system-prompt-override",
                "Translate-text-only.Never-use-tools.",
                "--verbatim",
            ]));
            a
        }
        _ => return Err(ProviderError::new("spawn", "Unknown CLI provider", id)),
    };
    if !model.is_empty() {
        args.extend([
            if id == "claude_cli" { "--model" } else { "-m" }.into(),
            model.into(),
        ]);
    }
    if !effort.is_empty() {
        match id {
            "codex_cli" => args.extend(["-c".into(), format!("model_reasoning_effort={effort}")]),
            "grok_cli" => args.extend(["--reasoning-effort".into(), effort.into()]),
            _ => args.extend(["--effort".into(), effort.into()]),
        }
    }
    if id == "codex_cli" {
        args.push("-".into());
    }
    Ok(args)
}
fn grok_events(raw: &str) -> impl Iterator<Item = Value> + '_ {
    raw.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .filter(|v| v["type"].is_string())
}
fn grok_stop(kind: &str) -> Option<&'static str> {
    match kind {
        "refusal" => Some("refused"),
        "max_tokens" | "max_turn_requests" | "max_turns_reached" => Some("truncated"),
        "cancelled" | "error" => Some("spawn"),
        _ => None,
    }
}
pub fn grok_finished(raw: &[u8]) -> bool {
    let tail = String::from_utf8_lossy(&raw[raw.len().saturating_sub(65536)..]);
    let found = grok_events(&tail)
        .any(|ev| ev["type"] == "end" || grok_stop(ev["type"].as_str().unwrap_or("")).is_some());
    found
}
pub fn parse_grok(raw: &str) -> Result<String, ProviderError> {
    let mut parts = String::new();
    for ev in grok_events(raw) {
        let kind = ev["type"].as_str().unwrap_or("");
        if let Some(stop) = grok_stop(kind) {
            return Err(ProviderError::new(
                stop,
                format!("Grok terminal event: {kind}"),
                "grok_cli",
            ));
        }
        if kind == "text" {
            if let Some(text) = ev["data"].as_str() {
                parts.push_str(text);
            }
        } else if kind == "end" {
            if ev["stopReason"].as_str().is_some_and(|r| r != "end_turn") {
                return Err(ProviderError::new(
                    "truncated",
                    "Grok ended before completing text",
                    "grok_cli",
                ));
            }
            return Ok(parts.trim().into());
        }
    }
    Err(ProviderError::new(
        "truncated",
        "Grok output has no terminal event",
        "grok_cli",
    ))
}
pub fn classify(result: &CliResult, body: &str, id: &str) -> Result<String, ProviderError> {
    if result.timed_out {
        return Err(ProviderError::new(
            "timeout",
            "No CLI response within deadline",
            id,
        ));
    }
    if result.code != Some(0) && !result.done_early {
        let both = format!("{}\n{}", result.stdout, result.stderr).to_ascii_lowercase();
        let signs = [
            (
                "spawn",
                vec![
                    "enoent",
                    "eacces",
                    "is not recognized as an internal or external command",
                ],
            ),
            (
                "limit",
                vec![
                    "hit your usage limit",
                    "usage limit reached",
                    "rate limit",
                    "quota exceeded",
                    "too many requests",
                    "insufficient balance",
                    "insufficient_balance",
                    "credit balance is too low",
                ],
            ),
            (
                "auth",
                vec![
                    "not logged in",
                    "not loggedin",
                    "login required",
                    "log in to continue",
                    "oauth token expired",
                    "oauth token has expired",
                    "session expired",
                    "invalid api key",
                    "authentication_error",
                    "401 unauthorized",
                    "not authenticated",
                ],
            ),
            (
                "refused",
                vec!["safeguards flagged", "usage policy", "content policy"],
            ),
        ];
        let kind = signs
            .iter()
            .find(|(_, s)| s.iter().any(|s| both.contains(s)))
            .map(|(k, _)| *k)
            .unwrap_or("spawn");
        // Do not return arbitrary child stderr: it can contain source text or credentials.
        return Err(ProviderError::new(
            kind,
            format!("CLI exited with status {:?}", result.code),
            id,
        ));
    }
    let text = body.trim();
    let lower = text.to_ascii_lowercase();
    if text.chars().count() < 400
        && [
            "i'm sorry",
            "i’m sorry",
            "i'm unable",
            "i’m unable",
            "i can't help",
            "i cannot help",
            "i won't help",
            "i can't assist",
            "i cannot assist",
            "sorry, i can't",
            "sorry, i cannot",
        ]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return Err(ProviderError::new("refused", "CLI returned a refusal", id));
    }
    if text.is_empty() {
        return Err(ProviderError::new("empty", "CLI returned no text", id));
    }
    Ok(text.into())
}
struct TempFolder(PathBuf);
impl TempFolder {
    fn new(prefix: &str) -> Result<Self, ProviderError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..100 {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "{prefix}_{}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                n
            ));
            match std::fs::create_dir(&p) {
                Ok(()) => return Ok(Self(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(ProviderError::new(
                        "spawn",
                        "CLI temporary directory could not be created",
                        "",
                    ))
                }
            }
        }
        Err(ProviderError::new(
            "spawn",
            "CLI temporary directory unavailable",
            "",
        ))
    }
}
impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn options(id: &str, cwd: &Path) -> RunOptions {
    let mut o = RunOptions::new(cwd);
    if id == "grok_cli" {
        o.first_byte = Some(Duration::from_secs(100));
        o.idle = Some(Duration::from_secs(30));
        o.grok_terminal = true;
        o.env.extend([
            ("GROK_DISABLE_AUTOUPDATER".into(), "1".into()),
            ("GROK_MEMORY".into(), "0".into()),
            ("GROK_SUBAGENTS".into(), "0".into()),
            ("GROK_WRITE_FILE".into(), "0".into()),
        ]);
    }
    o
}
pub async fn complete(
    config: &ProviderConfig,
    system: &str,
    user: &str,
    cancel: &CancelToken,
) -> Result<String, ProviderError> {
    let binary = resolve_bin(config).ok_or_else(|| {
        ProviderError::new(
            "spawn",
            "Official native CLI executable not found; .cmd/.bat wrappers are not executed",
            &config.id,
        )
    })?;
    let prompt = format!("{system}\n\n{user}");
    if prompt.len() > OUTPUT_CAP {
        return Err(ProviderError::new(
            "truncated",
            "Prompt exceeded 4 MiB",
            &config.id,
        ));
    }
    let folder = TempFolder::new("versora_cli")?;
    let prompt_file = folder.0.join("prompt.txt");
    let mut run = options(&config.id, &folder.0);
    if config.id == "grok_cli" {
        std::fs::write(&prompt_file, prompt).map_err(|_| {
            ProviderError::new("spawn", "CLI prompt file could not be written", &config.id)
        })?;
    } else {
        run.stdin_text = Some(prompt);
    }
    let args = preset_args(
        &config.id,
        config.model.trim(),
        config.effort.trim(),
        &prompt_file,
        &folder.0,
    )?;
    let result = run_cli(&binary, &args, run, cancel)
        .await
        .map_err(|mut e| {
            if e.provider.is_empty() {
                e.provider = config.id.clone();
            }
            e
        })?;
    let body = if config.id == "grok_cli"
        && !result.timed_out
        && (result.code == Some(0) || result.done_early)
    {
        parse_grok(&result.stdout)?
    } else {
        result.stdout.clone()
    };
    let text = classify(&result, &body, &config.id)?;
    confirm_call(config);
    Ok(text)
}
pub fn parse_status(id: &str, text: &str) -> (Option<bool>, Vec<String>) {
    match id {
        "claude_cli" => {
            let data = text
                .find('{')
                .zip(text.rfind('}'))
                .and_then(|(a, b)| serde_json::from_str::<Value>(&text[a..=b]).ok());
            (data.and_then(|v| v["loggedIn"].as_bool()), vec![])
        }
        "codex_cli" => {
            let l = text.to_ascii_lowercase();
            (
                if [
                    "not logged in",
                    "not loggedin",
                    "logged out",
                    "login required",
                ]
                .iter()
                .any(|s| l.contains(s))
                {
                    Some(false)
                } else if l.contains("logged in") {
                    Some(true)
                } else {
                    None
                },
                vec![],
            )
        }
        "grok_cli" => {
            let rx = regex::Regex::new(r"(?m)^[ \t]*[*-][ \t]+([A-Za-z0-9._:/-]{1,64})")
                .expect("constant regex");
            let models = rx
                .captures_iter(text)
                .map(|m| m[1].into())
                .collect::<Vec<_>>();
            let l = text.to_ascii_lowercase();
            (
                if l.contains("not authenticated") {
                    Some(false)
                } else if !models.is_empty() || l.contains("available models:") {
                    Some(true)
                } else {
                    None
                },
                models,
            )
        }
        _ => (None, vec![]),
    }
}
#[derive(Clone)]
struct CacheEntry {
    status: ProviderStatus,
    at: Instant,
}
#[derive(Default)]
struct StatusCache {
    status: HashMap<String, CacheEntry>,
    proofs: HashMap<String, Instant>,
    gates: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}
static CACHE: OnceLock<Mutex<StatusCache>> = OnceLock::new();
fn usable(status: &ProviderStatus, proof: Option<Instant>) -> bool {
    status.present
        && status.version_ok
        && (status.signed_in == Some(true)
            || (status.signed_in.is_none()
                && proof.is_some_and(|p| p.elapsed() < Duration::from_secs(300))))
}
pub fn confirm_call(config: &ProviderConfig) {
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    let key = format!("{}|{}", config.id, config.cli_path);
    cache.proofs.insert(key.clone(), now);
    if let Some(entry) = cache.status.get_mut(&key) {
        entry.status.usable = usable(&entry.status, Some(now));
        if entry.status.usable {
            entry.at = now;
        }
    }
}
pub fn forget_status() {
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    cache.status.clear();
    cache.proofs.clear();
}
pub async fn probe(config: &ProviderConfig, fresh: bool, cancel: &CancelToken) -> ProviderStatus {
    let requested_at = Instant::now();
    let key = format!("{}|{}", config.id, config.cli_path);
    let (gate, waiting) = {
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !fresh {
            if let Some(hit) = cache
                .status
                .get(&key)
                .filter(|e| e.at.elapsed() < Duration::from_secs(300))
            {
                return hit.status.clone();
            }
        }
        let gate = cache.gates.entry(key.clone()).or_default().clone();
        let waiting = gate.try_lock().is_err();
        (gate, waiting)
    };
    let _guard = tokio::select! {biased;_=crate::providers::cancelled(cancel)=>return missing_status(config,"CLI status probe stopped"),guard=gate.lock()=>guard};
    {
        let cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if waiting
            || !fresh
            || cache
                .status
                .get(&key)
                .is_some_and(|hit| hit.at >= requested_at)
        {
            if let Some(hit) = cache
                .status
                .get(&key)
                .filter(|e| e.at.elapsed() < Duration::from_secs(300))
            {
                return hit.status.clone();
            }
        }
    }
    let mut status = missing_status(
        config,
        "Official native executable not found; choose a native .exe path",
    );
    if let Some(binary) = resolve_bin(config) {
        status.present = true;
        if let Ok(folder) = TempFolder::new("versora_probe") {
            let mut o = options(&config.id, &folder.0);
            o.timeout = Duration::from_secs(15);
            o.first_byte = None;
            o.idle = None;
            o.grok_terminal = false;
            let version = run_cli(&binary, &["--version".into()], o.clone(), cancel).await;
            if let Ok(r) = version {
                status.version_ok = !r.timed_out && r.code == Some(0);
                if status.version_ok {
                    let rx = regex::Regex::new(r"\d+\.\d+(?:\.\d+)?").expect("constant regex");
                    status.version = rx
                        .find(&format!("{} {}", r.stdout, r.stderr))
                        .map(|m| m.as_str().into())
                        .unwrap_or_default();
                    let args = match config.id.as_str() {
                        "claude_cli" => vec!["auth".into(), "status".into()],
                        "codex_cli" => vec!["login".into(), "status".into()],
                        _ => vec!["models".into()],
                    };
                    if let Ok(r) = run_cli(&binary, &args, o, cancel).await {
                        if !r.timed_out {
                            let (signed, models) =
                                parse_status(&config.id, &format!("{}\n{}", r.stdout, r.stderr));
                            status.signed_in = if signed == Some(true) && r.code != Some(0) {
                                None
                            } else {
                                signed
                            };
                            status.models = models;
                        }
                    }
                }
            }
        }
    }
    status.detail = if status.signed_in == Some(false) {
        "CLI reports signed out"
    } else if status.version_ok && status.signed_in == Some(true) {
        "CLI reports signed in; paid translation and tool containment NOT_RUN"
    } else if status.present {
        "CLI found; usable sign-in has not been demonstrated"
    } else {
        &status.detail
    }
    .into();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    status.usable = usable(&status, cache.proofs.get(&key).copied());
    cache.status.insert(
        key,
        CacheEntry {
            status: status.clone(),
            at: Instant::now(),
        },
    );
    status
}
fn missing_status(config: &ProviderConfig, detail: &str) -> ProviderStatus {
    ProviderStatus{id:config.id.clone(),present:false,version:String::new(),version_ok:false,signed_in:None,usable:false,models:vec![],detail:redact(detail,&[]),limitations:match config.id.as_str(){"codex_cli"=>vec!["Read-only Codex retains shell and inherited MCP access; full no-tool isolation is not certified".into()],"grok_cli"=>vec!["Grok home isolation has not been ported; flags do not certify full containment".into()],_=>vec!["Live CLI translation and tool access containment NOT_RUN".into()]}}
}
