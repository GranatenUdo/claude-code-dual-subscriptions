use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    routing::post,
};
use claude_code_proxy::{
    config::{AliasProvider, resolve_openrouter_config},
    monitor::{MonitorHandle, RequestStatus},
    providers::openrouter::OpenRouterProvider,
    registry::Registry,
    server::app_with_options,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

#[tokio::test]
async fn unobservable_responses_and_stream_errors_fail_without_secret_diagnostics() {
    for (content_type, encoding, body) in [
        ("text/event-stream", "gzip", "opaque-compressed-body"),
        ("application/octet-stream", "identity", "opaque-body"),
        (
            "application/json",
            "identity",
            "{\"type\":\"message\",\"content\":[]}",
        ),
        (
            "text/event-stream",
            "identity",
            "data: {\"type\":\"error\",\"error\":{\"message\":\"rejected synthetic-key\"}}\n\n",
        ),
    ] {
        let upstream = upstream(Router::new().route(
            "/api/v1/messages",
            post(move || async move {
                (
                    [
                        ("content-type", content_type),
                        ("content-encoding", encoding),
                    ],
                    body,
                )
            }),
        ))
        .await;
        let monitor = MonitorHandle::default();
        let response = proxy(&upstream, &monitor, true)
            .oneshot(request("/v1/messages", "openrouter/vendor/model", true))
            .await
            .unwrap();
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            body.as_bytes()
        );
        let snapshot = monitor.snapshot();
        assert_eq!(snapshot.recent[0].status, RequestStatus::Failed);
        assert!(!format!("{snapshot:?}").contains("synthetic-key"));
    }
}

struct Upstream {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn upstream(app: Router) -> Upstream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Upstream { url, task }
}
fn proxy(upstream: &Upstream, monitor: &MonitorHandle, key: bool) -> Router {
    let dir = tempfile::tempdir().unwrap();
    let mut env =
        std::collections::HashMap::from([("CCP_OPENROUTER_BASE_URL".into(), upstream.url.clone())]);
    if key {
        env.insert("OPENROUTER_API_KEY".into(), "synthetic-key".into());
    }
    let provider = OpenRouterProvider::with_config(resolve_openrouter_config(&env, dir.path()));
    app_with_options(
        Arc::new(Registry::from_providers(
            AliasProvider::Anthropic,
            [Arc::new(provider) as Arc<dyn claude_code_proxy::provider::Provider>],
        )),
        Some(monitor.clone()),
        true,
    )
}
fn request(path: &str, model: &str, stream: bool) -> Request<Body> {
    Request::builder().method("POST").uri(path).header("content-type", "application/json")
        .header("authorization", "Bearer client-secret").header("x-api-key", "client-secret")
        .header("cookie", "client-cookie").header("x-claude-dual-capability", "private")
        .header("anthropic-beta", "test-beta")
        .body(Body::from(json!({"model":model,"stream":stream,"max_tokens":100,"messages":[],"future_field":{"keep":true}}).to_string())).unwrap()
}

#[tokio::test]
async fn json_preserves_fields_isolates_headers_and_records_usage_without_pricing() {
    let seen = Arc::new(Mutex::new(None));
    let capture = seen.clone();
    let upstream = upstream(Router::new().route("/api/v1/messages", post(move |headers: HeaderMap, body: Bytes| {
        let capture = capture.clone(); async move {
            *capture.lock().unwrap() = Some((headers, serde_json::from_slice::<Value>(&body).unwrap()));
            ([("set-cookie","never=forward"),("content-type","application/json")], r#"{"type":"message","model":"claude-opus-5-5","content":[],"usage":{"input_tokens":7,"output_tokens":3}}"#)
        }
    }))).await;
    let monitor = MonitorHandle::default();
    let response = proxy(&upstream, &monitor, true)
        .oneshot(request(
            "/v1/messages",
            "openrouter/vendor/model:free[1m]",
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key("set-cookie"));
    response.into_body().collect().await.unwrap();
    let (headers, body) = seen.lock().unwrap().take().unwrap();
    assert_eq!(headers["authorization"], "Bearer synthetic-key");
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(headers["anthropic-beta"], "test-beta");
    for header in ["x-api-key", "cookie", "x-claude-dual-capability"] {
        assert!(!headers.contains_key(header));
    }
    assert_eq!(body["model"], "vendor/model:free");
    assert_eq!(body["future_field"], json!({"keep":true}));
    let snapshot = monitor.snapshot();
    let row = &snapshot.recent[0];
    assert_eq!(row.status, RequestStatus::Completed);
    assert_eq!(row.input_tokens, Some(7));
    assert_eq!(row.output_tokens, Some(3));
    assert_eq!(row.provider.as_deref(), Some("openrouter"));
    assert!(row.usage_detail.cost.is_none());
}

#[tokio::test]
async fn broken_sse_is_relayed_but_never_reported_successful() {
    for text in [
        "data: {broken}\n\n".to_string(),
        "data: {\"type\":\"message_start\"}\n\n".into(),
        "data: {\"type\":\"message_stop\"}\n\ndata: {".into(),
        format!("data: {}\n\n", "x".repeat(2 * 1024 * 1024)),
    ] {
        let bytes = text.clone();
        let upstream = upstream(Router::new().route(
            "/api/v1/messages",
            post(move || {
                let bytes = bytes.clone();
                async move { ([("content-type", "text/event-stream")], bytes) }
            }),
        ))
        .await;
        let monitor = MonitorHandle::default();
        let response = proxy(&upstream, &monitor, true)
            .oneshot(request("/v1/messages", "openrouter/openrouter/auto", true))
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body.as_ref(), text.as_bytes());
        assert_eq!(monitor.snapshot().recent[0].status, RequestStatus::Failed);
    }
}

#[tokio::test]
async fn fragmented_stream_reports_usage_and_stops() {
    let upstream=upstream(Router::new().route("/api/v1/messages",post(|| async {
        let chunks=["data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":8}}}\n\nda", "ta: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\ndata: {\"type\":\"message_stop\"}\n\n"];
        ([("content-type","text/event-stream")],Body::from_stream(futures_util::stream::iter(chunks.map(|x|Ok::<_,Infallible>(Bytes::from_static(x.as_bytes()))))))
    }))).await;
    let monitor = MonitorHandle::default();
    proxy(&upstream, &monitor, true)
        .oneshot(request("/v1/messages", "openrouter/vendor/model", true))
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap();
    let snapshot = monitor.snapshot();
    assert_eq!(snapshot.recent[0].status, RequestStatus::Completed);
    assert_eq!(snapshot.recent[0].input_tokens, Some(8));
    assert_eq!(snapshot.recent[0].output_tokens, Some(4));
}

#[tokio::test]
async fn local_count_missing_key_and_unsupported_routes_never_call_upstream() {
    let upstream = upstream(Router::new().fallback(|| async {
        panic!("unexpected upstream call");
        #[allow(unreachable_code)]
        ""
    }))
    .await;
    let monitor = MonitorHandle::default();
    let app = proxy(&upstream, &monitor, false);
    let response = app
        .clone()
        .oneshot(request(
            "/v1/messages/count_tokens",
            "openrouter/vendor/model",
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(value["input_tokens"].is_u64());
    assert_eq!(
        app.clone()
            .oneshot(request("/v1/messages", "openrouter/vendor/model", false))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for path in ["/v1/chat/completions", "/v1/responses"] {
        let body = if path.ends_with("responses") {
            json!({"model":"openrouter/vendor/model","input":"hello"})
        } else {
            json!({"model":"openrouter/vendor/model","messages":[{"role":"user","content":"hello"}]})
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("does not support"));
    }
}

#[tokio::test]
async fn http_errors_are_bounded_redacted_and_redirects_are_not_followed() {
    for (status, expected) in [
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED),
        (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS),
        (StatusCode::TEMPORARY_REDIRECT, StatusCode::BAD_GATEWAY),
    ] {
        let upstream = upstream(Router::new().route(
            "/api/v1/messages",
            post(move || async move {
                (
                    status,
                    [
                        ("retry-after", "13"),
                        ("location", "http://127.0.0.1:1/never"),
                    ],
                    r#"{"error":{"message":"rejected synthetic-key"}}"#,
                )
            }),
        ))
        .await;
        let monitor = MonitorHandle::default();
        let response = proxy(&upstream, &monitor, true)
            .oneshot(request("/v1/messages", "openrouter/vendor/model", false))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["retry-after"], "13");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("synthetic-key"));
        assert!(text.contains("[redacted]"));
    }
}

#[tokio::test]
async fn original_json_model_cannot_defeat_dispatcher_auto_review_overrides() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let models = Arc::new(Mutex::new(Vec::new()));
    let seen = models.clone();
    let upstream=upstream(Router::new().route("/api/v1/messages",post(move |axum::Json(body):axum::Json<Value>| {
        let seen=seen.clone(); async move { seen.lock().unwrap().push(body["model"].as_str().unwrap().to_owned()); axum::Json(json!({"type":"message","content":[],"usage":{"input_tokens":1,"output_tokens":1}})) }
    }))).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"),json!({"autoReviewModel":"openrouter/openrouter/auto","openrouter":{"apiKey":"synthetic-key","baseUrl":upstream.url,"models":["vendor/favorite"]}}).to_string()).unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    assert_ne!(port, 18765);
    let _child = Child(
        std::process::Command::new(env!("CARGO_BIN_EXE_claude-code-proxy"))
            .env_clear()
            .env("HOME", dir.path())
            .env("XDG_CONFIG_HOME", dir.path())
            .env("CCP_CONFIG_DIR", dir.path())
            .args(["serve", "--no-monitor", "--port", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut ready = false;
    for _ in 0..200 {
        if client.get(format!("{base}/healthz")).send().await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(ready, "isolated proxy did not start");
    for model in ["gpt-6-astra", "openrouter/vendor/original"] {
        let response=client.post(format!("{base}/v1/messages")).json(&json!({"model":model,"stream":false,"messages":[],"system":[{"type":"text","text":"You are a security monitor for autonomous AI coding agents."}]})).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.bytes().await.unwrap();
    }
    assert_eq!(
        *models.lock().unwrap(),
        vec!["openrouter/auto", "openrouter/auto"]
    );
    let response=client.post(format!("{base}/v1/messages/count_tokens")).json(&json!({"model":"openrouter/vendor/original","messages":[],"system":[{"type":"text","text":"You are a security monitor for autonomous AI coding agents."}]})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap();
    assert_eq!(models.lock().unwrap().len(), 2);
    client
        .post(format!("{base}/v1/messages"))
        .json(&json!({"model":"openrouter/vendor/original","messages":[]}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(models.lock().unwrap().last().unwrap(), "vendor/original");
    let catalog: Value = client
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "openrouter/vendor/favorite")
    );
}

#[tokio::test]
async fn tools_and_foreign_reasoning_survive_round_trip() {
    let upstream=upstream(Router::new().route("/api/v1/messages",post(|axum::Json(body):axum::Json<Value>| async move {
        assert_eq!(body["messages"][0]["content"][0]["type"],"text");
        assert_eq!(body["messages"][0]["content"][1]["id"],"call_1");
        assert_eq!(body["messages"][1]["content"][0]["tool_use_id"],"call_1");
        assert_eq!(body["tools"][0]["input_schema"]["type"],"object");
        axum::Json(json!({"type":"message","content":[{"type":"tool_use","id":"call_2","name":"Read","input":{"path":"x"}}],"stop_reason":"tool_use"}))
    }))).await;
    let body = json!({"model":"openrouter/vendor/model","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"summary","signature":"ccp:codex:v1:opaque"},{"type":"tool_use","id":"call_1","name":"Read","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"ok"}]}],"tools":[{"name":"Read","input_schema":{"type":"object"}}]});
    let mut req = request("/v1/messages", "openrouter/vendor/model", false);
    *req.body_mut() = Body::from(body.to_string());
    let response = proxy(&upstream, &MonitorHandle::default(), true)
        .oneshot(req)
        .await
        .unwrap();
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["content"][0]["id"], "call_2");
    assert_eq!(value["stop_reason"], "tool_use");
}

#[tokio::test]
async fn http_200_json_error_diagnostics_do_not_echo_credentials() {
    let upstream = upstream(Router::new().route(
        "/api/v1/messages",
        post(|| async {
            axum::Json(json!({"type":"error","error":{"message":"rejected synthetic-key"}}))
        }),
    ))
    .await;
    let monitor = MonitorHandle::default();
    let response = proxy(&upstream, &monitor, true)
        .oneshot(request("/v1/messages", "openrouter/vendor/model", false))
        .await
        .unwrap();
    // The upstream response remains verbatim, but monitoring diagnostics are safe.
    response.into_body().collect().await.unwrap();
    let snapshot = monitor.snapshot();
    assert_eq!(snapshot.recent[0].status, RequestStatus::Failed);
    assert_eq!(
        snapshot.recent[0].error.as_deref(),
        Some("OpenRouter response error")
    );
}

#[tokio::test]
async fn invalid_ids_are_rejected_locally_and_native_aliases_are_not_claimed() {
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let called = hits.clone();
    let upstream = upstream(Router::new().fallback(move || {
        let called = called.clone();
        async move {
            called.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            "unexpected"
        }
    }))
    .await;
    let app = proxy(&upstream, &MonitorHandle::default(), true);
    for id in [
        "openrouter/",
        "openrouter/vendor/",
        "openrouter/bare",
        "openrouter/vendor/with space",
        "claude-opus-5-5",
        "gpt-6-astra",
        "opencode-go/model",
    ] {
        let response = app
            .clone()
            .oneshot(request("/v1/messages", id, false))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{id}");
    }
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn oversized_error_body_is_not_returned_or_accumulated_whole() {
    let upstream = upstream(Router::new().route(
        "/api/v1/messages",
        post(|| async { (StatusCode::BAD_REQUEST, "x".repeat(256 * 1024)) }),
    ))
    .await;
    let response = proxy(&upstream, &MonitorHandle::default(), true)
        .oneshot(request("/v1/messages", "openrouter/vendor/model", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() < 1024);
    assert!(String::from_utf8_lossy(&bytes).contains("OpenRouter upstream returned HTTP 400"));
}

#[tokio::test]
async fn dropping_downstream_cancels_upstream_stream() {
    struct Dropped(Arc<tokio::sync::Notify>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let dropped = Arc::new(tokio::sync::Notify::new());
    let signal = dropped.clone();
    let upstream = upstream(Router::new().route(
        "/api/v1/messages",
        post(move || {
            let guard = Dropped(signal.clone());
            async move {
                let stream =
                    futures_util::stream::unfold((0, guard), |(stage, guard)| async move {
                        if stage > 0 {
                            std::future::pending::<()>().await;
                        }
                        Some((
                            Ok::<_, Infallible>(Bytes::from_static(b"event: ping\ndata: {}\n\n")),
                            (stage + 1, guard),
                        ))
                    });
                (
                    [("content-type", "text/event-stream")],
                    Body::from_stream(stream),
                )
            }
        }),
    ))
    .await;
    let monitor = MonitorHandle::default();
    let response = proxy(&upstream, &monitor, true)
        .oneshot(request("/v1/messages", "openrouter/vendor/model", true))
        .await
        .unwrap();
    let mut body = response.into_body();
    body.frame().await.unwrap().unwrap();
    drop(body);
    tokio::time::timeout(std::time::Duration::from_secs(3), dropped.notified())
        .await
        .unwrap();
    assert!(monitor.snapshot().active.is_empty());
}
