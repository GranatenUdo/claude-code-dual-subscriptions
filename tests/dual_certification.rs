//! Test-only certification harness for `claude-code-dual launch`.
//!
//! Built only by `cargo test`. Run without arguments (as `cargo test` does) it
//! skips. `scripts/certify.py` runs it inside a bwrap namespace as
//! `dual_certification launch --claude /client [--resume ID]
//! --mock-record PATH`, where it performs the production preflight, scopes
//! the environment, and supervises the real client through the production
//! router, with both providers pointed at in-process mock upstreams. The
//! production binary has no way to change its upstreams.
//!
//! `dual_certification mock-serve --mock-record PATH` serves only the
//! Anthropic mock, so a directly started client can be compared.
use std::{
    ffi::OsString,
    io::Write,
    net::SocketAddr,
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use clap::Parser;
use claude_code_proxy::{
    dual::{
        cli::{Cli, Command},
        launch,
    },
    providers::{
        anthropic::AnthropicProvider,
        codex::{
            CodexProvider,
            auth::{
                manager::CodexAuthManager,
                token_store::{StoredAuth, file_store},
            },
            client::CodexHttpClient,
        },
    },
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const SYNTHETIC_OAUTH: &str = "Bearer sk-ant-oat01-SYNTHETIC-OFFLINE-ONLY";

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    let mode = args.get(1).and_then(|arg| arg.to_str()).map(str::to_owned);
    let record = take_option(&mut args, "--mock-record");
    match mode.as_deref() {
        Some("launch") => certify_launch(args, record),
        Some("mock-serve") => mock_serve(record),
        _ => {
            println!("dual_certification: skipped (probe-driven)");
            ExitCode::SUCCESS
        }
    }
}

fn take_option(args: &mut Vec<OsString>, name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.remove(index);
    (index < args.len()).then(|| args.remove(index).to_string_lossy().into_owned())
}

fn certify_launch(args: Vec<OsString>, record: Option<String>) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    let Command::Launch { claude, resume } = cli.command else {
        eprintln!("dual_certification: only launch is certified");
        return ExitCode::from(2);
    };
    // Same order as production: preflight and environment scoping happen
    // before any thread exists; mocks and providers come afterwards.
    let prepared = match unsafe { launch::preflight(claude, resume) } {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("Error: {error:#}");
            return ExitCode::from(1);
        }
    };
    unsafe { launch::scope_environment(&prepared) };
    if let Err(error) = write_synthetic_codex_auth() {
        eprintln!("Error: synthetic Codex auth: {error:#}");
        return ExitCode::from(1);
    }
    let recorder = Recorder::open(record);
    let mocks = Mocks::start(recorder.clone(), true);
    let anthropic = AnthropicProvider::with_client(
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("mock Anthropic client"),
        mocks.anthropic.clone(),
    );
    let codex = CodexProvider::with_client(Arc::new(CodexHttpClient::new_with_client(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("mock Codex client"),
        CodexAuthManager::new(file_store()),
        mocks.codex.clone().expect("Codex mock").to_string(),
    )));
    let code = launch::run_prepared(prepared, launch::router(anthropic, codex));
    recorder.write(json!({"event": "launcher_exit", "result": format!("{code:?}")}));
    match code {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn mock_serve(record: Option<String>) -> ExitCode {
    let recorder = Recorder::open(record);
    let mocks = Mocks::start(recorder, false);
    println!("MOCK_ANTHROPIC_URL={}", mocks.anthropic);
    let _ = std::io::stdout().flush();
    loop {
        std::thread::park();
    }
}

fn write_synthetic_codex_auth() -> anyhow::Result<()> {
    file_store().save_auth(StoredAuth {
        access: "SYNTHETIC-CODEX-ACCESS".to_string(),
        refresh: "SYNTHETIC-CODEX-NEVER-REFRESH".to_string(),
        // Far future, so no refresh is attempted.
        expires: 4_102_444_800_000,
        account_id: Some("acct_synthetic".to_string()),
    })
}

#[derive(Clone)]
struct Recorder(
    Option<Arc<Mutex<std::fs::File>>>,
    Arc<AtomicU64>,
    Arc<Mutex<std::collections::HashMap<String, usize>>>,
    Arc<Mutex<Option<String>>>,
);

impl Recorder {
    fn open(path: Option<String>) -> Self {
        let file = path.map(|path| {
            Arc::new(Mutex::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("record file"),
            ))
        });
        Self(
            file,
            Arc::new(AtomicU64::new(0)),
            Arc::default(),
            Arc::default(),
        )
    }

    fn pending_tool(&self) -> Option<String> {
        self.3
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_pending_tool(&self, id: &str) {
        *self
            .3
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(id.to_string());
    }

    fn clear_pending_tool(&self, id: &str) {
        let mut pending = self
            .3
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending.as_deref() == Some(id) {
            *pending = None;
        }
    }

    fn count(&self, marker: &str) -> usize {
        let mut counts = self
            .2
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = counts.entry(marker.to_string()).or_default();
        *count += 1;
        *count
    }

    fn next_seq(&self) -> u64 {
        self.1.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn write(&self, value: Value) {
        if let Some(file) = &self.0 {
            let mut file = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = writeln!(file, "{value}");
            let _ = file.flush();
        }
    }
}

struct Mocks {
    anthropic: reqwest::Url,
    codex: Option<reqwest::Url>,
}

impl Mocks {
    fn start(recorder: Recorder, codex: bool) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("mock runtime");
            runtime.block_on(async move {
                let anthropic = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let anthropic_addr = anthropic.local_addr().unwrap();
                let app = Router::new()
                    .route("/v1/messages", post(anthropic_messages))
                    .route("/v1/messages/count_tokens", post(anthropic_count_tokens))
                    .with_state(recorder.clone());
                tokio::spawn(async move {
                    axum::serve(anthropic, app).await.unwrap();
                });
                let codex_addr = if codex {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let addr = listener.local_addr().unwrap();
                    tokio::spawn(codex_websocket_server(listener, recorder.clone()));
                    Some(addr)
                } else {
                    None
                };
                tx.send((anthropic_addr, codex_addr)).unwrap();
                std::future::pending::<()>().await;
            });
        });
        let (anthropic, codex): (SocketAddr, Option<SocketAddr>) = rx.recv().unwrap();
        Self {
            anthropic: format!("http://{anthropic}").parse().unwrap(),
            codex: codex.map(|addr| {
                format!("http://{addr}/backend-api/codex/responses")
                    .parse()
                    .unwrap()
            }),
        }
    }
}

/// Synthetic markers in the last user message select the scripted reply.
fn marker(text: &str) -> Option<String> {
    let start = text.find("CERT-")?;
    let marker: String = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || *ch == '-')
        .collect();
    Some(marker.trim_end_matches('-').to_string())
}

fn block_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(text) => vec![text.clone()],
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn last_user(body: &Value) -> Option<&Value> {
    body["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
}

/// Claude merges and reorders history, so position cannot tell a tool
/// follow-up from a new prompt. The mock remembers the last tool use it issued
/// (unique id per issue): a request carrying that id's result is its follow-up.
fn pending_tool_result(body: &Value, pending: &str) -> Option<(bool, String)> {
    body["messages"].as_array()?.iter().find_map(|message| {
        message["content"].as_array()?.iter().find_map(|block| {
            (block["type"] == "tool_result" && block["tool_use_id"] == pending).then(|| {
                let content = match &block["content"] {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                (block["is_error"].as_bool().unwrap_or(false), content)
            })
        })
    })
}

/// The newest synthetic text: the last marker-bearing text block of the last
/// user message (earlier prompts can be folded into the same message).
fn newest_block(user: &Value) -> Option<String> {
    block_texts(&user["content"])
        .into_iter()
        .rev()
        .find(|text| text.contains("CERT-"))
}

enum Reply {
    Text {
        text: String,
        chunks: usize,
        delay: Duration,
    },
    Tool {
        kind: &'static str,
        name: &'static str,
        input: Value,
    },
    Error {
        status: StatusCode,
        kind: &'static str,
        message: &'static str,
        retry_after: Option<&'static str>,
    },
    Interrupted,
}

fn text(text: impl Into<String>) -> Reply {
    Reply::Text {
        text: text.into(),
        chunks: 3,
        delay: Duration::from_millis(60),
    }
}

fn script(body: &Value, recorder: &Recorder) -> (Option<String>, Reply) {
    let Some(user) = last_user(body) else {
        return (None, text("CERT-IDLE"));
    };
    if let Some(pending) = recorder.pending_tool()
        && let Some((is_error, content)) = pending_tool_result(body, &pending)
    {
        recorder.clear_pending_tool(&pending);
        let kind = pending
            .trim_start_matches("toolu_cert_")
            .split('_')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let outcome = match kind.as_str() {
            "READ" if content.contains("CERT-FIXTURE-CONTENT") => "OK",
            "MCP" if content.contains("CERT-MCP-PONG") => "OK",
            "AGENT" | "REVIEW" if content.contains("CERT-SUBAGENT-REPLY") => "OK",
            "AGENT" if content.contains("CERT-ASTRA-REPLY") => "OK",
            // Claude 2.1.300 runs agents in the background and reports the
            // result in a later task notification.
            "AGENT" | "REVIEW" if content.to_ascii_lowercase().contains("background") => "LAUNCHED",
            "BASH" | "SPAWN" if is_error => "ERR",
            "BASH" | "SPAWN" => "RAN",
            _ => "UNEXPECTED",
        };
        return (
            Some(format!("TOOL-RESULT-{kind}")),
            text(format!("CERT-{kind}-DONE-{outcome}")),
        );
    }
    let newest = newest_block(user).unwrap_or_default();
    // A background agent's result arrives as a task notification.
    if newest.contains("CERT-ASTRA-REPLY") {
        return (Some("AGENT-RESULT".into()), text("CERT-AGENT-RESULT-OK"));
    }
    if newest.contains("CERT-SUBAGENT-REPLY") {
        return (Some("REVIEW-RESULT".into()), text("CERT-REVIEW-RESULT-OK"));
    }
    let marker = marker(&newest);
    let reply = match marker.as_deref() {
        Some("CERT-TEXT-1") => text("CERT-REPLY-1"),
        Some("CERT-TEXT-2") => text("CERT-REPLY-2"),
        Some("CERT-SUBAGENT") => text("CERT-SUBAGENT-REPLY"),
        Some("CERT-SLOW") => Reply::Text {
            text: "CERT-SLOW-STREAM ".repeat(60),
            chunks: 60,
            delay: Duration::from_millis(250),
        },
        Some("CERT-READ") => Reply::Tool {
            kind: "read",
            name: "Read",
            input: json!({"file_path": "/home/test/project/fixture.txt"}),
        },
        Some("CERT-BASH") => Reply::Tool {
            kind: "bash",
            name: "Bash",
            input: json!({
                "command": "/usr/bin/touch /home/test/forbidden-tool-ran",
                "description": "Synthetic denied tool"
            }),
        },
        Some("CERT-SPAWN") => Reply::Tool {
            kind: "spawn",
            name: "Bash",
            input: json!({
                "command": "/usr/bin/sleep 300 > /dev/null 2>&1 & echo $! > /home/test/spawned.pid",
                "description": "Synthetic descendant"
            }),
        },
        Some("CERT-MCP") => Reply::Tool {
            kind: "mcp",
            name: "mcp__cert__echo",
            input: json!({"text": "CERT-MCP-PING"}),
        },
        Some("CERT-AGENT") => Reply::Tool {
            kind: "agent",
            name: "Agent",
            input: json!({
                "description": "Synthetic worker",
                "prompt": "Reply with CERT-SUBAGENT only.",
                "subagent_type": "general-purpose",
                "run_in_background": false
            }),
        },
        Some("CERT-REVIEW") => Reply::Tool {
            kind: "review",
            name: "Agent",
            input: json!({
                "description": "Synthetic reviewer",
                "prompt": "Reply with CERT-SUBAGENT only.",
                "subagent_type": "general-purpose",
                "model": "opus",
                "run_in_background": false
            }),
        },
        Some("CERT-ERR-401") => Reply::Error {
            status: StatusCode::UNAUTHORIZED,
            kind: "authentication_error",
            message: "CERT synthetic authentication failure",
            retry_after: None,
        },
        Some("CERT-ERR-429") => Reply::Error {
            status: StatusCode::TOO_MANY_REQUESTS,
            kind: "rate_limit_error",
            message: "CERT synthetic rate limit",
            retry_after: Some("1"),
        },
        Some("CERT-ERR-500") => Reply::Error {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            kind: "api_error",
            message: "CERT synthetic server error",
            retry_after: None,
        },
        Some("CERT-ERR-MODEL") => Reply::Error {
            status: StatusCode::NOT_FOUND,
            kind: "not_found_error",
            message: "CERT synthetic unsupported model",
            retry_after: None,
        },
        Some("CERT-ERR-INTERRUPT") => Reply::Interrupted,
        _ => text("CERT-IDLE"),
    };
    (marker, reply)
}

async fn anthropic_count_tokens(State(recorder): State<Recorder>) -> impl IntoResponse {
    recorder.write(json!({"event": "anthropic", "route": "/v1/messages/count_tokens"}));
    axum::Json(json!({"input_tokens": 100}))
}

async fn anthropic_messages(
    State(recorder): State<Recorder>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (marker, mut reply) = script(&body, &recorder);
    // A scripted tool use answered by anything but its result would loop.
    // Error markers are exempt: the client's own bounded retries are measured.
    if matches!(reply, Reply::Tool { .. })
        && recorder.count(marker.as_deref().unwrap_or_default()) > 20
    {
        reply = text("CERT-LOOP-GUARD");
    }
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let tools: Vec<String> = body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let seq = recorder.next_seq();
    recorder.write(json!({
        "event": "anthropic",
        "seq": seq,
        "route": "/v1/messages",
        "model": body["model"],
        "stream": body["stream"].as_bool().unwrap_or(false),
        "max_tokens": body["max_tokens"],
        "tool_count": tools.len(),
        "message_count": body["messages"].as_array().map_or(0, Vec::len),
        "marker": marker,
        "saved_oauth_preserved": header("authorization") == Some(SYNTHETIC_OAUTH),
        "capability_absent": header("x-claude-dual-capability").is_none(),
        "oauth_beta_present": header("anthropic-beta").is_some_and(|beta| beta.contains("oauth-2025-04-20")),
        "api_key_absent": header("x-api-key").is_none(),
        "agent_tool_present": tools.iter().any(|tool| tool == "Agent"),
        "mcp_tool_present": tools.iter().any(|tool| tool.starts_with("mcp__cert__")),
        "bash_tool_present": tools.iter().any(|tool| tool == "Bash"),
        "tail_shape": body["messages"].as_array().map(|messages| {
            messages
                .iter()
                .rev()
                .take(3)
                .map(|message| {
                    let blocks: Vec<String> = match &message["content"] {
                        Value::Array(blocks) => blocks
                            .iter()
                            .map(|block| {
                                format!(
                                    "{}:{}{}",
                                    block["type"].as_str().unwrap_or("?"),
                                    block["id"]
                                        .as_str()
                                        .or(block["tool_use_id"].as_str())
                                        .unwrap_or(""),
                                    if block["text"].as_str().is_some_and(|t| t.contains("CERT-")) {
                                        ":cert"
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .collect(),
                        Value::String(text) => vec![format!(
                            "string{}",
                            if text.contains("CERT-") { ":cert" } else { "" }
                        )],
                        _ => vec![],
                    };
                    json!({"role": message["role"], "blocks": blocks})
                })
                .collect::<Vec<_>>()
        }),
        "history_has_reply": body["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["role"] == "assistant"
                    && block_texts(&message["content"])
                        .iter()
                        .any(|text| text.contains("CERT-REPLY-"))
            })
        }),
    }));
    let model = body["model"]
        .as_str()
        .unwrap_or("claude-opus-5-5")
        .to_string();
    let stream = body["stream"].as_bool().unwrap_or(false);
    let tool_id = match &reply {
        Reply::Tool { kind, .. } => {
            let id = format!("toolu_cert_{kind}_{seq}");
            recorder.set_pending_tool(&id);
            Some(id)
        }
        _ => None,
    };
    match reply {
        Reply::Error {
            status,
            kind,
            message,
            retry_after,
        } => {
            let mut response = (
                status,
                [("content-type", "application/json")],
                json!({"type": "error", "error": {"type": kind, "message": message}}).to_string(),
            )
                .into_response();
            if let Some(retry_after) = retry_after {
                response
                    .headers_mut()
                    .insert("retry-after", retry_after.parse().unwrap());
            }
            response
        }
        Reply::Interrupted => sse_response(
            // The delay lets status and the first frames reach the client, so
            // the stream breaks mid-response rather than before the headers.
            sse_events(
                &model,
                &text("CERT-INTERRUPTED-PARTIAL"),
                None,
                Duration::from_millis(300),
                true,
            ),
            recorder,
            seq,
        ),
        reply if !stream => {
            let (content, stop) = match &reply {
                Reply::Text { text, .. } => (json!([{"type": "text", "text": text}]), "end_turn"),
                Reply::Tool { name, input, .. } => (
                    json!([{"type": "tool_use", "id": tool_id, "name": name, "input": input}]),
                    "tool_use",
                ),
                _ => unreachable!(),
            };
            axum::Json(json!({
                "id": format!("msg_cert_{seq}"),
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": content,
                "stop_reason": stop,
                "stop_sequence": null,
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }))
            .into_response()
        }
        reply => {
            let delay = match &reply {
                Reply::Text { delay, .. } => *delay,
                _ => Duration::from_millis(30),
            };
            sse_response(
                sse_events(&model, &reply, tool_id.as_deref(), delay, false),
                recorder,
                seq,
            )
        }
    }
}

type Event = (String, Value, Duration);

fn sse_events(
    model: &str,
    reply: &Reply,
    tool_id: Option<&str>,
    delay: Duration,
    interrupt: bool,
) -> Vec<Event> {
    let mut events: Vec<Event> = vec![(
        "message_start".into(),
        json!({"type": "message_start", "message": {
            "id": "msg_cert", "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 1}
        }}),
        Duration::ZERO,
    )];
    let stop = match reply {
        Reply::Text { text, chunks, .. } => {
            events.push((
                "content_block_start".into(),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                Duration::ZERO,
            ));
            let chars: Vec<char> = text.chars().collect();
            let size = chars.len().div_ceil((*chunks).max(1)).max(1);
            for piece in chars.chunks(size) {
                events.push((
                    "content_block_delta".into(),
                    json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece.iter().collect::<String>()}}),
                    delay,
                ));
                if interrupt {
                    return events;
                }
            }
            "end_turn"
        }
        Reply::Tool { name, input, .. } => {
            events.push((
                "content_block_start".into(),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": tool_id, "name": name, "input": {}}}),
                Duration::ZERO,
            ));
            events.push((
                "content_block_delta".into(),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
                delay,
            ));
            "tool_use"
        }
        _ => unreachable!(),
    };
    events.push((
        "content_block_stop".into(),
        json!({"type": "content_block_stop", "index": 0}),
        Duration::ZERO,
    ));
    events.push((
        "message_delta".into(),
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null}, "usage": {"output_tokens": 5}}),
        Duration::ZERO,
    ));
    events.push((
        "message_stop".into(),
        json!({"type": "message_stop"}),
        Duration::ZERO,
    ));
    events
}

/// Records whether the client consumed the whole stream or the request was
/// cancelled, which is how active-request cancellation is observed.
struct StreamWatch {
    recorder: Recorder,
    seq: u64,
    completed: bool,
}

impl StreamWatch {
    fn finish(&mut self, completed: bool) {
        self.completed = completed;
    }
}

impl Drop for StreamWatch {
    fn drop(&mut self) {
        self.recorder.write(json!({
            "event": "anthropic_stream_end",
            "seq": self.seq,
            "completed": self.completed,
        }));
    }
}

fn sse_response(events: Vec<Event>, recorder: Recorder, seq: u64) -> Response {
    let interrupted = events
        .last()
        .is_none_or(|(name, _, _)| name != "message_stop");
    let watch = StreamWatch {
        recorder,
        seq,
        completed: false,
    };
    let stream = futures_util::stream::unfold(
        (events.into_iter(), watch, interrupted),
        |(mut events, mut watch, interrupted)| async move {
            match events.next() {
                Some((name, data, delay)) => {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    let frame = format!("event: {name}\ndata: {data}\n\n");
                    Some((
                        Ok::<_, std::io::Error>(Bytes::from(frame)),
                        (events, watch, interrupted),
                    ))
                }
                None if interrupted => {
                    watch.finish(false);
                    Some((
                        Err(std::io::Error::other("synthetic interrupted stream")),
                        (events, watch, false),
                    ))
                }
                None => {
                    watch.finish(!interrupted);
                    None
                }
            }
        },
    );
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}

#[allow(clippy::result_large_err)]
async fn codex_websocket_server(listener: tokio::net::TcpListener, recorder: Recorder) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let authorization = Arc::new(Mutex::new(None::<String>));
            let seen = authorization.clone();
            let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &http::Request<()>, response| {
                    *seen.lock().unwrap() = request
                        .headers()
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    Ok(response)
                },
            )
            .await
            else {
                return;
            };
            let authorization = authorization.lock().unwrap().clone();
            while let Some(Ok(frame)) = socket.next().await {
                let Message::Text(text) = frame else {
                    if let Message::Ping(payload) = frame {
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    continue;
                };
                let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let seq = recorder.next_seq();
                recorder.write(json!({
                    "event": "codex",
                    "seq": seq,
                    "type": body["type"],
                    "model": body["model"],
                    "synthetic_codex_auth": authorization.as_deref() == Some("Bearer SYNTHETIC-CODEX-ACCESS"),
                }));
                let response_id = format!("resp_cert_{seq}");
                let events = [
                    json!({"type": "response.created", "response": {"id": response_id}}),
                    json!({"type": "response.output_item.added", "output_index": 0,
                        "item": {"type": "message", "id": format!("msg-{response_id}")}}),
                    json!({"type": "response.output_text.delta", "output_index": 0,
                        "delta": "CERT-ASTRA-REPLY"}),
                    json!({"type": "response.output_item.done", "output_index": 0,
                        "item": {"type": "message", "id": format!("msg-{response_id}")}}),
                    json!({"type": "response.completed", "response": {"id": response_id,
                        "usage": {"input_tokens": 50, "input_tokens_details": {"cached_tokens": 20},
                        "output_tokens": 4}}}),
                ];
                for event in events {
                    if socket.send(Message::Text(event.to_string())).await.is_err() {
                        return;
                    }
                }
            }
        });
    }
}
