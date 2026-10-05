use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use versora_engine::providers::*;
fn config(id: &str) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        ..Default::default()
    }
}
fn token() -> CancelToken {
    Arc::new(AtomicBool::new(false))
}
struct Sequence {
    answers: Mutex<VecDeque<Result<String, ProviderError>>>,
    called: Mutex<Vec<String>>,
}
impl Sequence {
    fn new(answers: Vec<Result<String, ProviderError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            called: Mutex::new(vec![]),
        }
    }
}
impl ProviderTransport for Sequence {
    fn complete<'a>(
        &'a self,
        cfg: &'a ProviderConfig,
        _: &'a str,
        _: &'a str,
        _: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        self.called.lock().unwrap().push(cfg.id.clone());
        let out = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("expected runtime call");
        Box::pin(async move { out })
    }
}
#[tokio::test]
async fn transient_original_plus_two_retries_then_ordered_failover() {
    let t = Sequence::new(vec![
        Err(ProviderError::new("limit", "busy", "a")),
        Err(ProviderError::new("empty", "empty", "a")),
        Err(ProviderError::new("timeout", "slow", "a")),
        Ok("answer".into()),
    ]);
    let runtime = ProviderRuntime::new(3, 1);
    let out = runtime
        .call_with(&t, &[config("a"), config("b")], "", "", &token())
        .await
        .unwrap();
    assert_eq!(out, "answer");
    assert_eq!(*t.called.lock().unwrap(), ["a", "a", "a", "b"]);
    let health = runtime.health();
    assert_eq!(health["a"].kind.as_deref(), Some("timeout"));
    assert!(health["b"].kind.is_none());
}
#[tokio::test]
async fn auth_is_immediate_and_explicit_provider_does_not_gain_auto() {
    let runtime = ProviderRuntime::new(3, 1);
    let t = Sequence::new(vec![
        Err(ProviderError::new("auth", "missing", "a")),
        Ok("B".into()),
    ]);
    assert_eq!(
        runtime
            .call_with(&t, &[config("a"), config("b")], "", "", &token())
            .await
            .unwrap(),
        "B"
    );
    assert_eq!(*t.called.lock().unwrap(), ["a", "b"]);
    let t = Sequence::new(vec![Err(ProviderError::new("auth", "missing", "a"))]);
    assert_eq!(
        runtime
            .call_with(&t, &[config("a")], "", "", &token())
            .await
            .unwrap_err()
            .kind,
        "auth"
    );
    assert_eq!(*t.called.lock().unwrap(), ["a"]);
}
#[tokio::test]
async fn truncation_is_returned_to_batch_split_without_retry_or_failover() {
    let t = Sequence::new(vec![Err(ProviderError::new("truncated", "cutoff", "a"))]);
    assert_eq!(
        ProviderRuntime::new(3, 1)
            .call_with(&t, &[config("a"), config("b")], "", "", &token())
            .await
            .unwrap_err()
            .kind,
        "truncated"
    );
    assert_eq!(*t.called.lock().unwrap(), ["a"]);
}
#[tokio::test]
async fn retry_after_over_thirty_fails_over_immediately() {
    let mut error = ProviderError::new("limit", "busy", "a");
    error.retry_after = Some(31.0);
    let t = Sequence::new(vec![Err(error), Ok("B".into())]);
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(500),
            ProviderRuntime::new(3, 1).call_with(&t, &[config("a"), config("b")], "", "", &token())
        )
        .await
        .unwrap()
        .unwrap(),
        "B"
    );
    assert_eq!(*t.called.lock().unwrap(), ["a", "b"]);
}
#[tokio::test]
async fn cancellation_interrupts_backoff_before_second_attempt() {
    let t = Sequence::new(vec![Err(ProviderError::new("limit", "busy", "a"))]);
    let cancel = token();
    let trigger = cancel.clone();
    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        trigger.store(true, Ordering::Release);
    });
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(500),
            ProviderRuntime::new(3, 1).call_with(&t, &[config("a")], "", "", &cancel)
        )
        .await
        .unwrap()
        .unwrap_err()
        .kind,
        "cancelled"
    );
    task.await.unwrap();
    assert_eq!(*t.called.lock().unwrap(), ["a"]);
}
#[derive(Default)]
struct Counters {
    total: usize,
    max_total: usize,
    per: HashMap<String, usize>,
    max_per: HashMap<String, usize>,
}
struct Delayed {
    counts: Mutex<Counters>,
}
impl ProviderTransport for Delayed {
    fn complete<'a>(
        &'a self,
        cfg: &'a ProviderConfig,
        _: &'a str,
        _: &'a str,
        cancel: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            {
                let mut c = self.counts.lock().unwrap();
                c.total += 1;
                c.max_total = c.max_total.max(c.total);
                let per = c.per.entry(cfg.id.clone()).or_default();
                *per += 1;
                let per = *per;
                let max = c.max_per.entry(cfg.id.clone()).or_default();
                *max = (*max).max(per);
            }
            cancellable_wait(Duration::from_millis(80), cancel).await?;
            {
                let mut c = self.counts.lock().unwrap();
                c.total -= 1;
                *c.per.get_mut(&cfg.id).unwrap() -= 1;
            }
            Ok("done".into())
        })
    }
}
#[tokio::test]
async fn shared_gates_enforce_both_limits_across_simultaneous_files() {
    let runtime = Arc::new(ProviderRuntime::new(2, 1));
    let transport = Arc::new(Delayed {
        counts: Mutex::new(Counters::default()),
    });
    let mut tasks = vec![];
    for n in 0..12 {
        let runtime = runtime.clone();
        let transport = transport.clone();
        tasks.push(tokio::spawn(async move {
            runtime
                .call_with(
                    transport.as_ref(),
                    &[config(["a", "b", "c"][n % 3])],
                    "",
                    "",
                    &token(),
                )
                .await
                .unwrap()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), "done");
    }
    let c = transport.counts.lock().unwrap();
    assert_eq!(c.max_total, 2);
    assert!(c.max_per.values().all(|n| *n == 1));
    assert_eq!(c.total, 0);
}
struct Held {
    entered: std::sync::atomic::AtomicUsize,
    release: tokio::sync::Semaphore,
}
impl ProviderTransport for Held {
    fn complete<'a>(
        &'a self,
        _: &'a ProviderConfig,
        _: &'a str,
        _: &'a str,
        _: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            self.entered.fetch_add(1, Ordering::Release);
            let _permit = self.release.acquire().await.unwrap();
            Ok("released".into())
        })
    }
}
#[tokio::test]
async fn cancellation_while_queued_never_reaches_transport() {
    let runtime = Arc::new(ProviderRuntime::new(1, 1));
    let transport = Arc::new(Held {
        entered: std::sync::atomic::AtomicUsize::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let task = {
        let runtime = runtime.clone();
        let transport = transport.clone();
        tokio::spawn(async move {
            runtime
                .call_with(transport.as_ref(), &[config("a")], "", "", &token())
                .await
                .unwrap()
        })
    };
    for _ in 0..100 {
        if transport.entered.load(Ordering::Acquire) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(transport.entered.load(Ordering::Acquire), 1);
    let cancel = token();
    let trigger = cancel.clone();
    let stop = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        trigger.store(true, Ordering::Release);
    });
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        runtime.call_with(transport.as_ref(), &[config("a")], "", "", &cancel),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(result.kind, "cancelled");
    assert_eq!(transport.entered.load(Ordering::Acquire), 1);
    transport.release.add_permits(1);
    assert_eq!(task.await.unwrap(), "released");
    stop.await.unwrap();
}

#[tokio::test]
async fn failed_demo_and_successful_real_fallback_do_not_mark_demo() {
    let runtime = ProviderRuntime::new(3, 1).for_operation();
    let transport = Sequence::new(vec![
        Err(ProviderError::new("auth", "Demo disabled", "demo")),
        Ok("real-provider fixture result".into()),
    ]);
    assert_eq!(
        runtime
            .call_with(
                &transport,
                &[config("demo"), config("openai")],
                "",
                "",
                &token()
            )
            .await
            .unwrap(),
        "real-provider fixture result"
    );
    assert_eq!(*transport.called.lock().unwrap(), ["demo", "openai"]);
    assert!(!runtime.used_demo());
}

#[tokio::test]
async fn successful_demo_fallback_marks_only_its_file_operation() {
    let shared = ProviderRuntime::new(3, 1);
    let operation = shared.for_operation();
    let sibling = shared.for_operation();
    let transport = Sequence::new(vec![
        Err(ProviderError::new("auth", "Key unavailable", "openai")),
        Ok("demo fallback fixture result".into()),
    ]);
    assert_eq!(
        operation
            .call_with(
                &transport,
                &[config("openai"), config("demo")],
                "",
                "",
                &token()
            )
            .await
            .unwrap(),
        "demo fallback fixture result"
    );
    assert!(operation.used_demo());
    assert!(operation.clone().used_demo());
    assert!(!shared.used_demo());
    assert!(!sibling.used_demo());
    assert!(!operation.for_operation().used_demo());
    // Shared health proves this detached usage tracker still reports the same provider runtime.
    assert_eq!(sibling.health()["openai"].kind.as_deref(), Some("auth"));
    assert!(sibling.health()["demo"].kind.is_none());
}

#[tokio::test]
async fn failed_or_cancelled_demo_never_creates_success_provenance() {
    for error in [
        ProviderError::new("auth", "disabled", "demo"),
        ProviderError::cancelled(),
        ProviderError::new("truncated", "incomplete", "demo"),
    ] {
        let operation = ProviderRuntime::new(3, 1).for_operation();
        let transport = Sequence::new(vec![Err(error)]);
        assert!(operation
            .call_with(&transport, &[config("demo")], "", "", &token())
            .await
            .is_err());
        assert!(!operation.used_demo());
    }
}

#[tokio::test]
async fn real_opt_in_offline_demo_fallback_records_success_after_tagged_response() {
    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("VERSORA_DEMO", value),
                None => std::env::remove_var("VERSORA_DEMO"),
            }
        }
    }
    let _restore = RestoreEnv(std::env::var_os("VERSORA_DEMO"));
    std::env::set_var("VERSORA_DEMO", "1");
    let operation = ProviderRuntime::new(3, 1).for_operation();
    assert!(!operation.used_demo());
    let output = operation
        .call(
            &[config("openai"), config("demo")],
            "",
            "Target language: fr\n\n1. Hello",
            &token(),
        )
        .await
        .unwrap();
    assert_eq!(output, "1. [fr] Hello");
    assert!(operation.used_demo());
    // Empty explicit API config fails locally before any request; the genuine Demo then completes.
    assert_eq!(operation.health()["openai"].kind.as_deref(), Some("auth"));
}
