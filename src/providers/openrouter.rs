use super::{
    messages_relay::{RelayPolicy, relay_with_policy},
    messages_request::prepare_body,
};
use crate::{
    anthropic::{
        json_error,
        schema::{CountTokensResponse, MessagesRequest},
    },
    config::{OpenRouterConfig, openrouter_config, valid_openrouter_model},
    provider::{CliHandlers, Provider, ProviderRequest, RequestContext, RequestEndpoint},
    registry::normalize_incoming_model,
};
use async_trait::async_trait;
use axum::{
    Json,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use std::time::Duration;

pub struct OpenRouterProvider {
    config: Result<OpenRouterConfig, String>,
    client: reqwest::Client,
}

pub fn advertised_models() -> Vec<String> {
    openrouter_config()
        .map(|cfg| {
            cfg.models
                .into_iter()
                .map(|id| format!("openrouter/{id}"))
                .collect()
        })
        .unwrap_or_default()
}

fn upstream_model(model: Option<&str>) -> Result<String, Box<Response>> {
    let normalized = normalize_incoming_model(model.unwrap_or_default());
    normalized.strip_prefix("openrouter/").filter(|id| valid_openrouter_model(id)).map(str::to_owned)
        .ok_or_else(|| Box::new(json_error(StatusCode::BAD_REQUEST, "invalid_request_error", "OpenRouter requires openrouter/<namespace>/<model> without whitespace or empty segments")))
}

impl OpenRouterProvider {
    pub fn new() -> Self {
        Self::with_config(openrouter_config())
    }

    pub fn with_config(config: Result<OpenRouterConfig, String>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(300))
            .build()
            .expect("OpenRouter HTTP client");
        Self { config, client }
    }

    fn config(&self) -> Result<&OpenRouterConfig, Box<Response>> {
        self.config.as_ref().map_err(|error| {
            Box::new(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                error.clone(),
            ))
        })
    }

    async fn send(&self, request: ProviderRequest, ctx: RequestContext) -> Response {
        if matches!(request.endpoint, RequestEndpoint::CountTokens) {
            return self.handle_count_tokens(request.body, ctx).await;
        }
        let cfg = match self.config() {
            Ok(cfg) => cfg,
            Err(response) => return *response,
        };
        let model = match upstream_model(request.body.model.as_deref()) {
            Ok(model) => model,
            Err(response) => return *response,
        };
        let Some(key) = &cfg.api_key else {
            return json_error(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "Configure CCP_OPENROUTER_API_KEY, OPENROUTER_API_KEY, or openrouter.apiKey for OpenRouter",
            );
        };
        let (parts, raw) = request.original.into_parts();
        let body = match prepare_body(raw, &model) {
            Ok(body) => body,
            Err(_) => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    "Invalid OpenRouter request JSON",
                );
            }
        };
        let mut url = cfg.base_url.clone();
        url.set_path(&format!("{}/messages", url.path().trim_end_matches('/')));
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert(
            "accept",
            HeaderValue::from_static(if request.body.stream {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
        headers.insert("accept-encoding", HeaderValue::from_static("identity"));
        headers.insert(
            "anthropic-version",
            parts
                .headers
                .get("anthropic-version")
                .cloned()
                .unwrap_or(HeaderValue::from_static("2023-06-01")),
        );
        if let Some(beta) = parts.headers.get("anthropic-beta") {
            headers.insert("anthropic-beta", beta.clone());
        }
        let mut auth = match HeaderValue::from_str(&format!("Bearer {key}")) {
            Ok(value) => value,
            Err(_) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "api_error",
                    "Invalid OpenRouter API key header",
                );
            }
        };
        auth.set_sensitive(true);
        headers.insert("authorization", auth);
        if let Some(monitor) = &ctx.monitor {
            monitor.model_resolved(&ctx.req_id, &model);
            monitor.upstream_started(&ctx.req_id);
        }
        if let Some(traffic) = &ctx.traffic {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) {
                traffic.write_json("020-upstream-request", &value);
            }
            traffic.write_json("021-upstream-request-metadata", &serde_json::json!({"provider":"openrouter","method":"POST","transport":"http","headers":{"authorization":"[redacted]"}}));
        }
        match self
            .client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => relay_with_policy(
                response,
                ctx,
                crate::pricing::PriceTier::Standard,
                RelayPolicy::OpenRouter {
                    streaming: request.body.stream,
                },
            ),
            Ok(response) => rejected_response(response, key).await,
            Err(_) => json_error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                "OpenRouter upstream request failed",
            ),
        }
    }
}

impl Default for OpenRouterProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "openrouter"
    }
    fn supported_models(&self) -> Vec<String> {
        self.config
            .as_ref()
            .map(|cfg| {
                cfg.models
                    .iter()
                    .map(|id| format!("openrouter/{id}"))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn cli(&self) -> &'static dyn CliHandlers {
        &OPENROUTER_CLI
    }
    async fn handle_request(&self, request: ProviderRequest, ctx: RequestContext) -> Response {
        self.send(request, ctx).await
    }
    async fn handle_messages(&self, _body: MessagesRequest, _ctx: RequestContext) -> Response {
        json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "OpenRouter requires the original Messages request",
        )
    }
    async fn handle_count_tokens(&self, body: MessagesRequest, ctx: RequestContext) -> Response {
        if let Err(response) = self.config() {
            return *response;
        }
        let model = match upstream_model(body.model.as_deref()) {
            Ok(model) => model,
            Err(response) => return *response,
        };
        let tokens = super::kimi::count_tokens::count_tokens(&body);
        if let Some(monitor) = &ctx.monitor {
            monitor.model_resolved(&ctx.req_id, model);
            monitor.stream_progress_estimated_input(&ctx.req_id, 0, 0, Some(tokens), None);
            monitor.cost_estimated(&ctx.req_id);
        }
        Json(CountTokensResponse {
            input_tokens: tokens,
        })
        .into_response()
    }
}

async fn rejected_response(response: reqwest::Response, key: &str) -> Response {
    let upstream_status = response.status();
    let status = if upstream_status.is_redirection() {
        StatusCode::BAD_GATEWAY
    } else {
        upstream_status
    };
    let retry_after = response.headers().get("retry-after").cloned();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while bytes.len() < 64 * 1024 {
        match stream.next().await {
            Some(Ok(chunk)) => {
                let remaining = 64 * 1024 - bytes.len();
                bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            _ => break,
        }
    }
    let message = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|s| s.as_str())
                .map(str::to_owned)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            format!(
                "OpenRouter upstream returned HTTP {}",
                upstream_status.as_u16()
            )
        })
        .replace(key, "[redacted]");
    let kind = match status {
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN | StatusCode::PAYMENT_REQUIRED => "permission_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        s if s.is_client_error() => "invalid_request_error",
        _ => "api_error",
    };
    let mut response = json_error(status, kind, message);
    if let Some(value) = retry_after {
        response.headers_mut().insert("retry-after", value);
    }
    response
}

struct OpenRouterCli;
static OPENROUTER_CLI: OpenRouterCli = OpenRouterCli;
impl CliHandlers for OpenRouterCli {
    fn login(&self) -> anyhow::Result<()> {
        anyhow::bail!(
            "Set CCP_OPENROUTER_API_KEY, OPENROUTER_API_KEY, or openrouter.apiKey; OpenRouter has no proxy-managed login"
        )
    }
    fn device(&self) -> anyhow::Result<()> {
        self.login()
    }
    fn logout(&self) -> anyhow::Result<()> {
        anyhow::bail!(
            "Remove your OpenRouter key from its environment or configuration source; the proxy does not store login credentials"
        )
    }
    fn status(&self) -> anyhow::Result<()> {
        let config = openrouter_config().map_err(anyhow::Error::msg)?;
        match config.api_key_source {
            Some(source) => {
                println!("OpenRouter API key configured via {source} (not validated remotely)")
            }
            None => println!("OpenRouter API key not configured"),
        }
        Ok(())
    }
}
