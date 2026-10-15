use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::post,
};
use claude_code_proxy::{
    config::AliasProvider,
    monitor::{MonitorHandle, RequestStatus},
    providers::anthropic::AnthropicProvider,
    registry::Registry,
    server::app_with_monitor,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tower::ServiceExt;

#[tokio::test]
async fn local_capability_never_reaches_native_upstream() {
    use claude_code_proxy::dual::security::{CAPABILITY_HEADER, Capability, protect};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let upstream = upstream(Router::new().route(
        "/v1/messages",
        post(move |headers: HeaderMap| {
            let observed = observed.clone();
            async move {
                assert!(!headers.contains_key(CAPABILITY_HEADER));
                assert_eq!(headers["authorization"], "Bearer claude-subscription");
                observed.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"id":"mock", "content":[]}))
            }
        }),
    ))
    .await;
    let cap = Capability::generate();
    let monitor = MonitorHandle::default();
    let app = protect(proxy(&upstream, &monitor), cap.clone());
    for supplied in [None, Some("wrong")] {
        let mut req = request(
            "/v1/messages",
            r#"{"model":"claude-opus-5-5","messages":[]}"#,
        );
        if let Some(value) = supplied {
            req.headers_mut()
                .insert(CAPABILITY_HEADER, value.parse().unwrap());
        }
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let mut req = request(
        "/v1/messages",
        r#"{"model":"claude-opus-5-5","messages":[]}"#,
    );
    let header = cap.child_header();
    req.headers_mut().insert(
        CAPABILITY_HEADER,
        header.split_once(": ").unwrap().1.parse().unwrap(),
    );
    assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::OK);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

struct Upstream {
    url: reqwest::Url,
    task: JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn upstream(app: Router) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Upstream { url, task }
}

fn proxy(upstream: &Upstream, monitor: &MonitorHandle) -> Router {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let provider: Arc<dyn claude_code_proxy::provider::Provider> =
        Arc::new(AnthropicProvider::with_client(client, upstream.url.clone()));
    let registry = Arc::new(Registry::from_providers(
        AliasProvider::Anthropic,
        [provider],
    ));
    app_with_monitor(registry, Some(monitor.clone()))
}

fn request(path: &str, body: impl Into<Body>) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer claude-subscription")
        .header(
            "anthropic-beta",
            "oauth-2025-04-20,prompt-caching-2024-07-31",
        )
        .header("anthropic-version", "2023-06-01")
        .body(body.into())
        .unwrap()
}

#[tokio::test]
async fn relays_native_body_credentials_query_and_json_usage() {
    let captured = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let upstream = upstream(Router::new().route("/v1/messages", post(move |req: Request<Body>| {
        let sink = sink.clone();
        async move {
            let (parts, body) = req.into_parts();
            let body = axum::body::to_bytes(body, usize::MAX).await.unwrap();
            *sink.lock().await = Some((parts, body));
            ([("request-id", "req_anthropic"), ("content-type", "application/json")],
             r#"{"type":"message","usage":{"input_tokens":17,"output_tokens":8},"content":[{"type":"thinking","signature":"native","thinking":"summary"}]}"#)
        }
    }))).await;
    let monitor = MonitorHandle::default();
    let raw = r#"{ "model":"claude-opus-5", "messages":[], "future_field":{"enabled":true} }"#;
    let response = proxy(&upstream, &monitor)
        .oneshot(request("/v1/messages?beta=true", raw))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["request-id"], "req_anthropic");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"signature\":\"native\"")
    );
    let captured = captured.lock().await;
    let (parts, body) = captured.as_ref().unwrap();
    assert_eq!(body, raw.as_bytes());
    assert_eq!(parts.uri.query(), Some("beta=true"));
    assert_eq!(parts.headers["authorization"], "Bearer claude-subscription");
    assert_eq!(parts.headers["anthropic-version"], "2023-06-01");
    assert!(
        parts.headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .contains("oauth")
    );
    let snapshot = monitor.snapshot();
    assert_eq!(snapshot.recent[0].provider.as_deref(), Some("anthropic"));
    assert_eq!(snapshot.recent[0].input_tokens, Some(17));
    assert_eq!(snapshot.recent[0].output_tokens, Some(8));
    assert_eq!(snapshot.recent[0].status, RequestStatus::Completed);
}

#[tokio::test]
async fn streams_before_completion_and_observes_fragmented_usage() {
    let release = Arc::new(Notify::new());
    let released = release.clone();
    let upstream = upstream(Router::new().route("/v1/messages", post(move || {
        let released = released.clone();
        async move {
            let chunks = futures_util::stream::unfold(0, move |stage| {
                let released = released.clone();
                async move {
                    let data = match stage {
                        0 => "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":7,\"output_tokens\":0}}}\n\nevent: message_delta\nda",
                        1 => { released.notified().await; "ta: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n" },
                        _ => return None,
                    };
                    Some((Ok::<_, Infallible>(Bytes::from_static(data.as_bytes())), stage + 1))
                }
            });
            ([("content-type", "text/event-stream")], Body::from_stream(chunks))
        }
    }))).await;
    let monitor = MonitorHandle::default();
    let response = proxy(&upstream, &monitor)
        .oneshot(request(
            "/v1/messages",
            r#"{"model":"claude-opus-5","stream":true,"messages":[]}"#,
        ))
        .await
        .unwrap();
    let mut body = response.into_body();
    let first = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    assert!(first.starts_with(b"event: message_start"));
    assert_eq!(monitor.snapshot().active[0].input_tokens, Some(7));
    release.notify_one();
    let rest = body.collect().await.unwrap().to_bytes();
    assert!(rest.ends_with(b"data: {\"type\":\"message_stop\"}\n\n"));
    let snapshot = monitor.snapshot();
    assert_eq!(snapshot.recent[0].output_tokens, Some(9));
    assert_eq!(snapshot.recent[0].status, RequestStatus::Completed);
}

#[tokio::test]
async fn token_counting_applies_the_same_model_and_thinking_conversion() {
    let upstream = upstream(Router::new().route("/v1/messages/count_tokens", post(|headers: HeaderMap, axum::Json(body): axum::Json<Value>| async move {
        assert_eq!(headers["authorization"], "Bearer claude-subscription");
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["messages"][0]["content"][0], json!({"type":"text","text":"<previous_reasoning>\nGPT summary\n</previous_reasoning>"}));
        axum::Json(json!({"input_tokens":42}))
    }))).await;
    let monitor = MonitorHandle::default();
    let body = json!({"model":"opus[1m]","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"GPT summary","signature":"ccp:codex:v1:cnNfMQ:ciphertext"}]}]});
    let response = proxy(&upstream, &monitor)
        .oneshot(request(
            "/v1/messages/count_tokens?beta=true",
            body.to_string(),
        ))
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"input_tokens":42})
    );
    assert_eq!(monitor.snapshot().recent[0].input_tokens, Some(42));
}

#[tokio::test]
async fn preserves_upstream_errors_and_does_not_follow_redirects() {
    for (status, body) in [
        (
            StatusCode::UNAUTHORIZED,
            r#"{"type":"error","error":{"type":"authentication_error","message":"expired"}}"#,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"quota"}}"#,
        ),
        (StatusCode::TEMPORARY_REDIRECT, "redirect"),
    ] {
        let upstream = upstream(Router::new().route(
            "/v1/messages",
            post(move || async move {
                (
                    status,
                    [
                        ("retry-after", "13"),
                        ("location", "http://127.0.0.1:1/never"),
                        ("request-id", "real-id"),
                    ],
                    body,
                )
                    .into_response()
            }),
        ))
        .await;
        let monitor = MonitorHandle::default();
        let response = proxy(&upstream, &monitor)
            .oneshot(request(
                "/v1/messages",
                r#"{"model":"claude-opus-5","messages":[]}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["retry-after"], "13");
        assert_eq!(response.headers()["request-id"], "real-id");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            body.as_bytes()
        );
    }
}

#[tokio::test]
async fn streamed_errors_remain_verbatim_and_fail_the_dashboard_request() {
    let raw = "event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}\n\n";
    let upstream = upstream(Router::new().route(
        "/v1/messages",
        post(move || async move { ([("content-type", "text/event-stream")], raw) }),
    ))
    .await;
    let monitor = MonitorHandle::default();
    let response = proxy(&upstream, &monitor)
        .oneshot(request(
            "/v1/messages",
            r#"{"model":"claude-opus-5","messages":[],"stream":true}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        raw.as_bytes()
    );
    let snapshot = monitor.snapshot();
    assert_eq!(snapshot.recent[0].status, RequestStatus::Failed);
    assert_eq!(snapshot.recent[0].error.as_deref(), Some("overloaded"));
}

#[tokio::test]
async fn dropping_downstream_cancels_upstream_stream() {
    struct Dropped(Arc<Notify>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let dropped = Arc::new(Notify::new());
    let signal = dropped.clone();
    let upstream = upstream(Router::new().route(
        "/v1/messages",
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
    let response = proxy(&upstream, &monitor)
        .oneshot(request(
            "/v1/messages",
            r#"{"model":"claude-opus-5","messages":[],"stream":true}"#,
        ))
        .await
        .unwrap();
    let mut body = response.into_body();
    body.frame().await.unwrap().unwrap();
    drop(body);
    tokio::time::timeout(Duration::from_secs(3), dropped.notified())
        .await
        .unwrap();
    assert!(monitor.snapshot().active.is_empty());
}

struct IsolatedProvider;

#[async_trait::async_trait]
impl claude_code_proxy::provider::Provider for IsolatedProvider {
    fn name(&self) -> &'static str {
        "codex"
    }
    fn supported_models(&self) -> Vec<String> {
        vec!["gpt-6-sol".into()]
    }
    fn cli(&self) -> &'static dyn claude_code_proxy::provider::CliHandlers {
        panic!("not used")
    }
    async fn handle_request(
        &self,
        request: claude_code_proxy::provider::ProviderRequest,
        _: claude_code_proxy::provider::RequestContext,
    ) -> axum::response::Response {
        assert!(request.original.headers().is_empty());
        assert!(request.original.uri().query().is_none());
        (StatusCode::OK, "isolated").into_response()
    }
    async fn handle_messages(
        &self,
        _: claude_code_proxy::anthropic::schema::MessagesRequest,
        _: claude_code_proxy::provider::RequestContext,
    ) -> axum::response::Response {
        panic!("not used")
    }
    async fn handle_count_tokens(
        &self,
        _: claude_code_proxy::anthropic::schema::MessagesRequest,
        _: claude_code_proxy::provider::RequestContext,
    ) -> axum::response::Response {
        panic!("not used")
    }
}

#[tokio::test]
async fn switching_codex_to_claude_keeps_authentication_isolated() {
    let upstream = upstream(Router::new().route(
        "/v1/messages",
        post(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer claude-subscription");
            "native"
        }),
    ))
    .await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let providers: Vec<Arc<dyn claude_code_proxy::provider::Provider>> = vec![
        Arc::new(AnthropicProvider::with_client(client, upstream.url.clone())),
        Arc::new(IsolatedProvider),
    ];
    let app = app_with_monitor(
        Arc::new(Registry::from_providers(
            AliasProvider::Anthropic,
            providers,
        )),
        None,
    );
    for (model, expected) in [
        ("gpt-6-sol", "isolated"),
        ("opus", "native"),
        ("gpt-6-sol", "isolated"),
        ("claude-opus-5", "native"),
    ] {
        let mut req = request(
            "/v1/messages?api_key=client-secret",
            json!({"model":model,"messages":[]}).to_string(),
        );
        req.headers_mut()
            .insert("x-api-key", "client-key".parse().unwrap());
        req.headers_mut().insert(
            "x-claude-code-session-id",
            "dual-subscription-auth-test".parse().unwrap(),
        );
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            expected
        );
    }
    let response = app
        .oneshot(request(
            "/v1/messages/count_tokens?token=client-secret",
            r#"{"model":"gpt-6-sol","messages":[]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn native_schema_validation_and_error_bytes_belong_to_anthropic() {
    let raw = r#"{"model":"claude-opus-5","max_tokens":-1,"messages":"invalid","future":true}"#;
    let error = r#"{"type":"error","error":{"type":"invalid_request_error","message":"native validation"}}"#;
    let upstream = upstream(Router::new().route(
        "/v1/messages",
        post(move |body: Bytes| async move {
            assert_eq!(body, raw);
            (
                StatusCode::BAD_REQUEST,
                [("content-type", "application/json")],
                error,
            )
        }),
    ))
    .await;
    let response = proxy(&upstream, &MonitorHandle::default())
        .oneshot(request("/v1/messages", raw))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        error
    );
}
