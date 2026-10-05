use serde_json::json;
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use versora_engine::{
    cli,
    providers::*,
    translate::{self, BatchCaller, TranslationOptions},
};

#[test]
fn request_shapes_defaults_and_secret_redaction() {
    for (id, model, suffix) in [
        ("openai", "gpt-4.1-mini", "/chat/completions"),
        ("xai", "grok-3-mini", "/chat/completions"),
        ("anthropic", "claude-haiku-4-5", "/v1/messages"),
        ("gemini", "gemini-2.5-flash", ":generateContent"),
    ] {
        let cfg = ProviderConfig {
            id: id.into(),
            api_key: "synthetic-protocol-key".into(),
            ..Default::default()
        };
        let request = request_spec(&cfg, "SYS", "USER").unwrap();
        assert!(request.url.ends_with(suffix));
        assert!(!request.body.to_string().contains("temperature"));
        if id != "gemini" {
            assert_eq!(request.body["model"], model);
        }
        assert!(!format!("{cfg:?}").contains(&cfg.api_key));
        assert!(!serde_json::to_string(&cfg).unwrap().contains(&cfg.api_key));
    }
    assert_eq!(
        redact(
            "oops synthetic-protocol-key sk-12345678abcdef",
            &["synthetic-protocol-key"]
        ),
        "oops **** ****"
    );
    for id in ["openai", "anthropic", "gemini", "xai"] {
        assert_eq!(
            request_spec(
                &ProviderConfig {
                    id: id.into(),
                    ..Default::default()
                },
                "",
                ""
            )
            .err()
            .unwrap()
            .kind,
            "auth"
        );
    }
}
#[test]
fn api_stop_reasons_are_never_partial_success() {
    assert_eq!(
        parse_response(
            "openai",
            &json!({"choices":[{"message":{"content":"text"},"finish_reason":"stop"}]})
        )
        .unwrap(),
        "text"
    );
    assert_eq!(
        parse_response(
            "openai",
            &json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]})
        )
        .err()
        .unwrap()
        .kind,
        "truncated"
    );
    assert_eq!(
        parse_response(
            "xai",
            &json!({"choices":[{"message":{"content":"partial"},"finish_reason":"content_filter"}]})
        )
        .err()
        .unwrap()
        .kind,
        "refused"
    );
    assert_eq!(parse_response("anthropic",&json!({"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"A"},{"type":"text","text":"B"}],"stop_reason":"end_turn"})).unwrap(),"AB");
    assert_eq!(
        parse_response(
            "anthropic",
            &json!({"content":[{"type":"text","text":"partial"}],"stop_reason":"max_tokens"})
        )
        .err()
        .unwrap()
        .kind,
        "truncated"
    );
    assert_eq!(parse_response("gemini",&json!({"candidates":[{"content":{"parts":[{"text":"thought","thought":true},{"text":"answer"}]},"finishReason":"STOP"}]})).unwrap(),"answer");
    for reason in [
        "SAFETY",
        "RECITATION",
        "PROHIBITED_CONTENT",
        "BLOCKLIST",
        "SPII",
    ] {
        assert_eq!(
            parse_response("gemini", &json!({"candidates":[{"finishReason":reason}]}))
                .err()
                .unwrap()
                .kind,
            "refused"
        );
    }
    assert_eq!(
        parse_response(
            "gemini",
            &json!({"promptFeedback":{"blockReason":"SAFETY"}})
        )
        .err()
        .unwrap()
        .kind,
        "refused"
    );
    assert_eq!(
        parse_response("openai", &json!({})).err().unwrap().kind,
        "empty"
    );
}
#[test]
fn endpoint_and_retry_status_policy() {
    for url in [
        "http://api.openai.com",
        "https://127.0.0.1",
        "https://chatgpt.com",
        "https://claude.ai",
        "https://grok.com",
        "https://evil.invalid",
        "https://user:password@api.openai.com",
        "https://api.openai.com/#fragment",
    ] {
        assert!(validate_endpoint(url, "openai").is_err(), "{url}");
    }
    assert!(validate_endpoint("https://api.openai.com/v1", "openai").is_ok());
    for (code, kind) in [
        (401, "auth"),
        (403, "auth"),
        (408, "timeout"),
        (504, "timeout"),
        (429, "limit"),
        (500, "limit"),
        (503, "limit"),
        (400, "spawn"),
        (404, "spawn"),
    ] {
        assert_eq!(kind_for_status(code), kind);
    }
    assert_eq!(backoff(0, None).unwrap().as_secs(), 1);
    assert_eq!(backoff(1, None).unwrap().as_secs(), 2);
    assert_eq!(backoff(1, Some(7.0)).unwrap().as_secs(), 7);
    assert!(backoff(0, Some(31.0)).is_none());
    assert!(backoff(0, Some(f64::NAN)).is_none());
}
#[test]
fn cli_argv_has_no_document_no_permission_widening_and_sign_in_is_tristate() {
    use std::path::Path;
    for id in ["claude_cli", "codex_cli", "grok_cli"] {
        let args = cli::preset_args(
            id,
            "model-1",
            "high",
            Path::new("C:/temp/prompt.txt"),
            Path::new("C:/temp/work"),
        )
        .unwrap();
        assert!(!args.join(" ").contains("DOCUMENT TEXT"));
        for denied in [
            "--yolo",
            "--full-auto",
            "bypassPermissions",
            "danger-full-access",
            "workspace-write",
        ] {
            assert!(!args.join(" ").contains(denied));
        }
        assert!(cli::preset_args(id, "a;calc.exe", "", Path::new("p"), Path::new("w")).is_err());
    }
    assert_eq!(
        cli::parse_status("claude_cli", r#"{"loggedIn":true}"#).0,
        Some(true)
    );
    assert_eq!(cli::parse_status("claude_cli", "unknown").0, None);
    assert_eq!(
        cli::parse_status("codex_cli", "Not logged in").0,
        Some(false)
    );
    assert_eq!(
        cli::parse_status("codex_cli", "Logged in using ChatGPT").0,
        Some(true)
    );
    assert_eq!(
        cli::parse_status("grok_cli", "not authenticated\n- model-a").0,
        Some(false)
    );
    assert_eq!(cli::parse_grok("{\"type\":\"text\",\"data\":\"Hello\"}\n{\"type\":\"end\",\"stopReason\":\"end_turn\"}\n").unwrap(),"Hello");
    assert_eq!(
        cli::parse_grok("{\"type\":\"text\",\"data\":\"partial\"}\n")
            .err()
            .unwrap()
            .kind,
        "truncated"
    );
    let result = cli::CliResult {
        code: Some(0),
        stdout: String::new(),
        stderr: "rate limit quoted within translated document".into(),
        timed_out: false,
        done_early: false,
    };
    assert_eq!(
        cli::classify(&result, "translation", "codex_cli").unwrap(),
        "translation"
    );
}

struct ScriptCaller {
    responses: Mutex<VecDeque<Result<String, ProviderError>>>,
    calls: Mutex<Vec<String>>,
}
impl ScriptCaller {
    fn new(responses: Vec<Result<String, ProviderError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(vec![]),
        }
    }
}
impl BatchCaller for ScriptCaller {
    fn call<'a>(
        &'a self,
        _: &'a str,
        user: &'a str,
        _: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        self.calls.lock().unwrap().push(user.into());
        let reply = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("one response per expected call");
        Box::pin(async move { reply })
    }
}
fn token() -> CancelToken {
    Arc::new(AtomicBool::new(false))
}
#[tokio::test]
async fn stable_dedup_blank_units_and_shape_restoration() {
    let caller = ScriptCaller::new(vec![Ok("1. Bonjour\n2. Monde".into())]);
    let source = vec![
        " Hello \n".into(),
        "".into(),
        "World".into(),
        " Hello \n".into(),
    ];
    let events = Arc::new(Mutex::new(vec![]));
    let e = events.clone();
    let out = translate::translate_texts_with(
        &caller,
        &source,
        &TranslationOptions::default(),
        "system",
        &token(),
        Some(Arc::new(move |a, b| e.lock().unwrap().push((a, b)))),
    )
    .await
    .unwrap();
    assert_eq!(out, vec![" Bonjour \n", "", "Monde", " Bonjour \n"]);
    assert_eq!(caller.calls.lock().unwrap().len(), 1);
    assert_eq!(*events.lock().unwrap(), vec![(0, 1), (1, 1)]);
}
#[tokio::test]
async fn malformed_and_truncated_batches_split_in_order() {
    let caller = ScriptCaller::new(vec![
        Err(ProviderError::new("truncated", "limit", "test")),
        Ok("malformed".into()),
        Ok("1. A".into()),
        Ok("1. B".into()),
        Ok("1. C\n2. D\n3. E".into()),
    ]);
    let source = ["a", "b", "c", "d", "e"].map(str::to_owned);
    let out = translate::translate_batch_with(&caller, "", "header", &source, &token(), true)
        .await
        .unwrap();
    assert_eq!(out, ["A", "B", "C", "D", "E"]);
    let calls = caller.calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert!(calls[0].contains("5. e"));
    assert!(calls[1].contains("2. b"));
    assert!(calls[4].contains("3. e"));
}
#[tokio::test]
async fn single_reflow_reasks_once_then_fails_closed() {
    let caller = ScriptCaller::new(vec![
        Ok("1. bad\nextra".into()),
        Ok("1. still bad\nextra".into()),
    ]);
    let err =
        translate::translate_batch_with(&caller, "", "header", &["one".into()], &token(), true)
            .await
            .unwrap_err();
    assert_eq!(err.kind, "structure");
    let calls = caller.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].contains("exactly one numbered item"));
}
#[tokio::test]
async fn multiline_reflow_retries_source_lines_without_flattening() {
    let caller = ScriptCaller::new(vec![
        Ok("1. reflowed paragraph".into()),
        Ok("1. Un\n2. Deux".into()),
    ]);
    let out = translate::translate_batch_with(
        &caller,
        "",
        "header",
        &["  One  \n\tTwo\n".into()],
        &token(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(out, vec!["  Un  \n\tDeux\n"]);
}
#[tokio::test]
async fn cancellation_before_transport_makes_zero_calls() {
    let caller = ScriptCaller::new(vec![]);
    let cancel = Arc::new(AtomicBool::new(true));
    assert_eq!(
        translate::translate_texts_with(
            &caller,
            &["hello".into()],
            &TranslationOptions::default(),
            "",
            &cancel,
            None
        )
        .await
        .unwrap_err()
        .kind,
        "cancelled"
    );
    assert!(caller.calls.lock().unwrap().is_empty());
}
