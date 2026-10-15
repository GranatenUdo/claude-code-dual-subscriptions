use axum::{body::Body, response::Response};
use futures_util::StreamExt;
use http::{HeaderMap, header};
use serde_json::Value;

use crate::{
    monitor::TokenUsage,
    openai_compat::stream::SseDecoder,
    pricing::PriceTier,
    provider::{RequestContext, ResponseOutcome},
};

use super::headers::forwarded_headers;

const MAX_JSON_OBSERVATION_BYTES: usize = 2 * 1024 * 1024;

/// Usage of one Anthropic response, accumulated across `message_start`,
/// `message_delta` and JSON bodies. Anthropic reports exact counts, which only
/// grow, so every field keeps its largest observed value.
#[derive(Debug, Default)]
pub(super) struct UsageAccumulator {
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
    cache_write_5m: Option<u64>,
    cache_write_1h: Option<u64>,
    model: Option<String>,
    fallback_model: Option<String>,
    fast: bool,
    estimated: bool,
    finalized: bool,
}

impl UsageAccumulator {
    pub(super) fn new(request_tier: PriceTier) -> Self {
        Self {
            fast: request_tier == PriceTier::Fast,
            ..Self::default()
        }
    }

    fn observe_usage(&mut self, usage: &Value) -> bool {
        let mut observed = false;
        for (field, value) in [
            (&mut self.input, usage.get("input_tokens")),
            (&mut self.output, usage.get("output_tokens")),
            (&mut self.cache_read, usage.get("cache_read_input_tokens")),
            (
                &mut self.cache_creation,
                usage.get("cache_creation_input_tokens"),
            ),
            (
                &mut self.cache_write_5m,
                usage.pointer("/cache_creation/ephemeral_5m_input_tokens"),
            ),
            (
                &mut self.cache_write_1h,
                usage.pointer("/cache_creation/ephemeral_1h_input_tokens"),
            ),
        ] {
            if let Some(value) = value.and_then(Value::as_u64) {
                *field = Some(field.map_or(value, |current| current.max(value)));
                observed = true;
            }
        }
        // The reported speed is authoritative over the requested one.
        if let Some(speed) = usage.get("speed").and_then(Value::as_str) {
            self.fast = speed == "fast";
        }
        // A server-side fallback bills several attempts; top-level usage covers
        // only the returned one, so the cost becomes an estimate.
        if let Some(iterations) = usage.get("iterations").and_then(Value::as_array)
            && (iterations.len() > 1
                || iterations.iter().any(|iteration| {
                    iteration.get("type").and_then(Value::as_str) == Some("fallback_message")
                }))
        {
            self.estimated = true;
        }
        observed
    }

    fn observe_value(&mut self, value: &Value) -> bool {
        let mut observed = false;
        for usage in [value.get("usage"), value.pointer("/message/usage")]
            .into_iter()
            .flatten()
        {
            observed |= self.observe_usage(usage);
        }
        let model = value
            .pointer("/message/model")
            .or_else(|| {
                (value.get("type").and_then(Value::as_str) == Some("message"))
                    .then(|| value.get("model"))
                    .flatten()
            })
            .and_then(Value::as_str);
        if let Some(model) = model {
            self.model = Some(model.to_owned());
        }
        let blocks = value.get("content_block").into_iter().chain(
            value
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        );
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("fallback")
                && let Some(model) = block.pointer("/to/model").and_then(Value::as_str)
            {
                self.fallback_model = Some(model.to_owned());
            }
        }
        observed
    }

    /// Cache writes split by duration. A total without a matching breakdown
    /// bills its unattributed part as 5-minute writes.
    fn cache_writes(&self) -> (Option<u64>, Option<u64>) {
        if self.cache_write_5m.is_none() && self.cache_write_1h.is_none() {
            return (self.cache_creation, None);
        }
        let write_1h = self.cache_write_1h.unwrap_or(0);
        let write_5m = self.cache_write_5m.unwrap_or(0);
        match self.cache_creation {
            Some(total) if total != write_5m.saturating_add(write_1h) => {
                (Some(total.saturating_sub(write_1h)), Some(write_1h))
            }
            _ => (Some(write_5m), Some(write_1h)),
        }
    }

    pub(super) fn token_usage(&self) -> TokenUsage {
        let (cache_write_5m, cache_write_1h) = self.cache_writes();
        TokenUsage {
            input: self.input,
            cache_read: self.cache_read,
            cache_write_5m,
            cache_write_1h,
            output: self.output,
        }
    }

    fn publish_final(&mut self, ctx: &RequestContext) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let Some(monitor) = &ctx.monitor else {
            return;
        };
        let tier = if self.fast {
            PriceTier::Fast
        } else {
            PriceTier::Standard
        };
        if let Some(model) = self.fallback_model.as_ref().or(self.model.as_ref()) {
            monitor.pricing_resolved(&ctx.req_id, model, tier);
        }
        if self.estimated {
            monitor.cost_estimated(&ctx.req_id);
        }
        let usage = self.token_usage();
        if usage != TokenUsage::default() {
            monitor.usage_final(&ctx.req_id, usage);
        }
    }
}

enum Observation {
    Events {
        decoder: SseDecoder,
        completed: bool,
    },
    Json(Vec<u8>),
    Opaque,
}

impl Observation {
    fn for_headers(headers: &HeaderMap) -> Self {
        if headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|encoding| encoding != "identity")
        {
            return Self::Opaque;
        }
        let media_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        match media_type {
            Some("text/event-stream") => Self::Events {
                decoder: SseDecoder::default(),
                completed: false,
            },
            Some("application/json") => Self::Json(Vec::new()),
            _ => Self::Opaque,
        }
    }

    fn receive(
        &mut self,
        bytes: &[u8],
        ctx: &RequestContext,
        outcome: &ResponseOutcome,
        usage: &mut UsageAccumulator,
    ) {
        match self {
            Self::Events { decoder, completed } => {
                match decoder.push(bytes) {
                    Ok(events) => {
                        for event in events {
                            observe_value(&event.data, ctx, outcome, usage);
                            if event.data.get("type").and_then(Value::as_str)
                                == Some("message_stop")
                            {
                                *completed = true;
                                usage.publish_final(ctx);
                            }
                        }
                    }
                    Err(_) => {
                        // Observation must never alter the relayed bytes or grow without bound.
                        // Usage seen so far may be partial, so its cost is an estimate.
                        *self = Self::Opaque;
                        if let Some(monitor) = &ctx.monitor {
                            monitor.cost_estimated(&ctx.req_id);
                        }
                    }
                }
                if let Some(monitor) = &ctx.monitor {
                    monitor.stream_progress(&ctx.req_id, bytes.len() as u64, 1, None, None);
                }
            }
            Self::Json(buffer)
                if buffer.len().saturating_add(bytes.len()) <= MAX_JSON_OBSERVATION_BYTES =>
            {
                buffer.extend_from_slice(bytes);
            }
            Self::Json(_) => *self = Self::Opaque,
            Self::Opaque => {}
        }
    }

    fn finish(
        self,
        ctx: &RequestContext,
        outcome: &ResponseOutcome,
        usage: &mut UsageAccumulator,
        success: bool,
    ) {
        match self {
            Self::Json(buffer) => {
                if let Ok(value) = serde_json::from_slice(&buffer) {
                    observe_value(&value, ctx, outcome, usage);
                }
            }
            Self::Events {
                completed: false, ..
            } => {
                outcome.fail("Anthropic stream ended before message_stop".into());
            }
            Self::Opaque if success => {
                if let Some(monitor) = &ctx.monitor {
                    monitor.usage_unmetered(&ctx.req_id);
                }
            }
            _ => {}
        }
        // Usage seen before an interrupted stream still counts.
        usage.publish_final(ctx);
    }
}

fn observe_value(
    value: &Value,
    ctx: &RequestContext,
    outcome: &ResponseOutcome,
    usage_accumulator: &mut UsageAccumulator,
) {
    if value.get("type").and_then(Value::as_str) == Some("error") {
        outcome.fail(
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Anthropic stream error")
                .to_owned(),
        );
    }
    let observed = usage_accumulator.observe_value(value);
    if let Some(monitor) = &ctx.monitor {
        if observed {
            monitor.usage_observed(&ctx.req_id, usage_accumulator.token_usage());
        }
        if let Some(tokens) = value.get("input_tokens").and_then(Value::as_u64) {
            monitor.usage_updated(&ctx.req_id, Some(tokens), None);
        }
        if value.get("type").and_then(Value::as_str) == Some("message_start") {
            monitor.generation_started(&ctx.req_id);
        }
    }
}

pub(super) fn relay(
    upstream: reqwest::Response,
    ctx: RequestContext,
    request_tier: PriceTier,
) -> Response {
    let observation = Observation::for_headers(upstream.headers());
    let outcome = ResponseOutcome::default();
    let success = upstream.status().is_success();
    let response = http::Response::builder().status(upstream.status());
    let headers = forwarded_headers(upstream.headers());
    let usage = UsageAccumulator::new(request_tier);
    let stream = futures_util::stream::unfold(
        (
            upstream.bytes_stream(),
            observation,
            ctx,
            outcome.clone(),
            usage,
        ),
        move |(mut upstream, mut observation, ctx, outcome, mut usage)| async move {
            match upstream.next().await {
                Some(chunk) => {
                    if let Ok(bytes) = &chunk {
                        observation.receive(bytes, &ctx, &outcome, &mut usage);
                    }
                    Some((chunk, (upstream, observation, ctx, outcome, usage)))
                }
                None => {
                    observation.finish(&ctx, &outcome, &mut usage, success);
                    None
                }
            }
        },
    );
    let response = headers.iter().fold(response, |response, (name, value)| {
        response.header(name, value)
    });
    response
        .extension(outcome)
        .body(Body::from_stream(stream))
        .expect("valid upstream response")
}
