//! Genuine Rust test-harness subprocesses and descendants; no Python helper or live paid CLI.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use versora_engine::{
    cli::{self, RunOptions},
    providers::CancelToken,
};

#[test]
#[ignore]
fn provider_process_helper() {
    use std::{
        io::{Read, Write},
        process::{Command, Stdio},
    };
    let mode = std::env::var("VERSORA_TEST_HELPER_MODE")
        .expect("helper is only spawned by provider process tests");
    if let Ok(folder) = std::env::var("VERSORA_TEST_HELPER_DIR") {
        std::fs::write(
            PathBuf::from(folder).join(format!("{mode}.pid")),
            std::process::id().to_string(),
        )
        .unwrap();
    }
    match mode.as_str() {
        "echo" => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).unwrap();
            assert!(std::env::var("SYNTHETIC_API_KEY").is_err());
            assert!(std::env::var("NEXT_PUBLIC_SYNTHETIC_TOKEN").is_err());
            println!("ECHO:{text}");
            eprintln!("ERR:drained");
        }
        "tree" | "child" => {
            let next = if mode == "tree" {
                "child"
            } else {
                "grandchild"
            };
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "provider_process_helper",
                    "--nocapture",
                ])
                .env("VERSORA_TEST_HELPER_MODE", next)
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            loop {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        "grandchild" | "idle" => loop {
            std::thread::sleep(Duration::from_millis(50));
        },
        "exit-with-child" => {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "provider_process_helper",
                    "--nocapture",
                ])
                .env("VERSORA_TEST_HELPER_MODE", "grandchild")
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let folder = PathBuf::from(std::env::var("VERSORA_TEST_HELPER_DIR").unwrap());
            std::fs::write(folder.join("detached.pid"), child.id().to_string()).unwrap();
            for _ in 0..100 {
                if folder.join("grandchild.pid").is_file() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("pipe-holding child did not start");
        }
        "noise" => {
            let block = [b'x'; 65536];
            loop {
                std::io::stdout().write_all(&block).unwrap();
                std::io::stderr().write_all(&block).unwrap();
            }
        }
        _ => panic!("unknown helper mode"),
    }
}
fn setup(mode: &str) -> (PathBuf, RunOptions, Vec<String>) {
    let folder = std::env::temp_dir().join(format!(
        "versora_rust_process_{}_{}_{}",
        std::process::id(),
        mode,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&folder).unwrap();
    let mut options = RunOptions::new(&folder);
    options
        .env
        .push(("VERSORA_TEST_HELPER_MODE".into(), mode.into()));
    options.env.push((
        "VERSORA_TEST_HELPER_DIR".into(),
        folder.as_os_str().to_owned(),
    ));
    (
        folder,
        options,
        vec![
            "--ignored".into(),
            "--exact".into(),
            "provider_process_helper".into(),
            "--nocapture".into(),
        ],
    )
}
fn cancel() -> CancelToken {
    Arc::new(AtomicBool::new(false))
}
#[tokio::test]
async fn native_stdout_stderr_and_stdin_drained() {
    let (folder, mut options, args) = setup("echo");
    options.stdin_text = Some("DOCUMENT $() & literal".into());
    options.timeout = Duration::from_secs(5);
    options.env.extend([
        ("SYNTHETIC_API_KEY".into(), "fake-test-credential".into()),
        (
            "NEXT_PUBLIC_SYNTHETIC_TOKEN".into(),
            "fake-test-credential".into(),
        ),
    ]);
    assert!(cli::child_env()
        .iter()
        .all(|(k, _)| !versora_engine::providers::is_secret_name(&k.to_string_lossy())));
    let result = cli::run_cli(&std::env::current_exe().unwrap(), &args, options, &cancel())
        .await
        .unwrap();
    assert_eq!(result.code, Some(0));
    assert!(result.stdout.contains("ECHO:DOCUMENT $() & literal"));
    assert!(result.stderr.contains("ERR:drained"));
    assert!(!result.stdout.contains("fake-test-credential"));
    std::fs::remove_dir_all(folder).unwrap();
}
#[tokio::test]
async fn output_cap_covers_both_pipes_and_does_not_hang() {
    let (folder, mut options, args) = setup("noise");
    options.timeout = Duration::from_secs(5);
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        cli::run_cli(&std::env::current_exe().unwrap(), &args, options, &cancel()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(result.kind, "truncated");
    std::fs::remove_dir_all(folder).unwrap();
}
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == 259
    }
}
#[cfg(windows)]
async fn wait_tree(folder: &std::path::Path) {
    for _ in 0..200 {
        if ["tree", "child", "grandchild"]
            .iter()
            .all(|s| folder.join(format!("{s}.pid")).is_file())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("genuine subprocess descendants did not launch");
}
#[cfg(windows)]
fn assert_tree_gone(folder: &std::path::Path) {
    for name in ["tree", "child", "grandchild"] {
        let pid = std::fs::read_to_string(folder.join(format!("{name}.pid")))
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(
            !alive(pid),
            "{name} PID {pid} survived Windows job shutdown"
        );
    }
}
#[cfg(windows)]
#[tokio::test]
async fn cancellation_kills_native_child_and_grandchild_and_closes_inherited_pipes() {
    let (folder, mut options, args) = setup("tree");
    options.timeout = Duration::from_secs(10);
    let cancellation = cancel();
    let trigger = cancellation.clone();
    let wait = folder.clone();
    let task = tokio::spawn(async move {
        wait_tree(&wait).await;
        trigger.store(true, Ordering::Release);
    });
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        cli::run_cli(
            &std::env::current_exe().unwrap(),
            &args,
            options,
            &cancellation,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    task.await.unwrap();
    assert_eq!(result.kind, "cancelled");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_tree_gone(&folder);
    std::fs::remove_dir_all(folder).unwrap();
}
#[cfg(windows)]
#[tokio::test]
async fn deadline_kills_native_child_and_grandchild() {
    let (folder, mut options, args) = setup("tree");
    options.timeout = Duration::from_secs(2);
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        cli::run_cli(&std::env::current_exe().unwrap(), &args, options, &cancel()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.timed_out);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_tree_gone(&folder);
    std::fs::remove_dir_all(folder).unwrap();
}
#[cfg(windows)]
#[tokio::test]
async fn successful_parent_exit_closes_pipe_holding_descendant() {
    let (folder, mut options, args) = setup("exit-with-child");
    options.timeout = Duration::from_secs(5);
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        cli::run_cli(&std::env::current_exe().unwrap(), &args, options, &cancel()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.code, Some(0));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let pid = std::fs::read_to_string(folder.join("detached.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        !alive(pid),
        "pipe-holding descendant PID {pid} survived normal parent exit"
    );
    std::fs::remove_dir_all(folder).unwrap();
}
