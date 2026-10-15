use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use crate::pricing::{self, PriceFamily, PriceTier, PricedUsage};

mod mock;
pub mod remote;
pub mod snapshot;

pub use mock::{MockMonitor, mock_state};

const DEFAULT_RECENT_LIMIT: usize = 200;
pub const SESSION_TOKEN_BUCKET_SECS: u64 = 10;
pub const SESSION_OUTPUT_HISTORY_BUCKETS: u64 = 60 * 60 / SESSION_TOKEN_BUCKET_SECS;
const SESSION_USAGE_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const SESSION_PRUNE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    Messages,
    CountTokens,
    Responses,
    ChatCompletions,
    Images,
    Transcriptions,
}

impl EndpointKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::CountTokens => "count_tokens",
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::Images => "images",
            Self::Transcriptions => "transcriptions",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Started,
    ProviderSelected,
    Compacting,
    Upstream,
    Streaming,
    Completed,
    Failed,
}

impl RequestStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::ProviderSelected => "selected",
            Self::Compacting => "compacting",
            Self::Upstream => "upstream",
            Self::Streaming => "streaming",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub enum MonitorEvent {
    RequestStarted {
        request_id: String,
        session_id: Option<String>,
        session_seq: Option<u64>,
        endpoint: EndpointKind,
    },
    ProjectResolved {
        request_id: String,
        project: String,
    },
    SessionSequenceResolved {
        request_id: String,
        session_seq: u64,
    },
    ProviderSelected {
        request_id: String,
        provider: String,
        model: String,
        effort: Option<String>,
    },
    ModelResolved {
        request_id: String,
        model: String,
    },
    CompactionStarted {
        request_id: String,
    },
    UpstreamStarted {
        request_id: String,
    },
    GenerationStarted {
        request_id: String,
    },
    TrafficCapturePath {
        request_id: String,
        path: PathBuf,
    },
    StreamProgress {
        request_id: String,
        bytes: u64,
        chunks: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        /// The input count is a local estimate, not provider-reported usage.
        input_estimated: bool,
    },
    UsageUpdated {
        request_id: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    /// Progress usage with cache detail; counts only ever grow.
    UsageObserved {
        request_id: String,
        usage: TokenUsage,
    },
    /// Provider-reported final usage; replaces every field it carries,
    /// including earlier local estimates.
    UsageFinal {
        request_id: String,
        usage: TokenUsage,
    },
    /// The upstream model and service tier the request is billed at.
    PricingResolved {
        request_id: String,
        model: String,
        tier: PriceTier,
    },
    /// Usage was counted locally, so the cost is only an estimate.
    CostEstimated {
        request_id: String,
    },
    /// The response could not be observed, so no usage is known.
    UsageUnmetered {
        request_id: String,
    },
    RequestCompleted {
        request_id: String,
        http_status: u16,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    RequestFailed {
        request_id: String,
        http_status: Option<u16>,
        error: String,
    },
    RequestAbandoned {
        request_id: String,
        error: String,
    },
}

impl MonitorEvent {
    fn request_id(&self) -> &str {
        match self {
            Self::RequestStarted { request_id, .. }
            | Self::ProjectResolved { request_id, .. }
            | Self::SessionSequenceResolved { request_id, .. }
            | Self::ProviderSelected { request_id, .. }
            | Self::ModelResolved { request_id, .. }
            | Self::CompactionStarted { request_id }
            | Self::UpstreamStarted { request_id }
            | Self::GenerationStarted { request_id }
            | Self::TrafficCapturePath { request_id, .. }
            | Self::StreamProgress { request_id, .. }
            | Self::UsageUpdated { request_id, .. }
            | Self::UsageObserved { request_id, .. }
            | Self::UsageFinal { request_id, .. }
            | Self::PricingResolved { request_id, .. }
            | Self::CostEstimated { request_id }
            | Self::UsageUnmetered { request_id }
            | Self::RequestCompleted { request_id, .. }
            | Self::RequestFailed { request_id, .. }
            | Self::RequestAbandoned { request_id, .. } => request_id,
        }
    }
}

/// Mutable usage fields of one request, whether still active or completed.
struct RequestUsageMut<'a> {
    session_id: Option<String>,
    input_tokens: &'a mut Option<u64>,
    output_tokens: &'a mut Option<u64>,
    detail: &'a mut UsageDetail,
    generation_started_instant: Option<Instant>,
    generation_finished_at: &'a mut Option<SystemTime>,
    generation_duration: &'a mut Option<Duration>,
    /// Set for completed requests, whose output history is recorded per delta.
    finished_at: Option<SystemTime>,
}

impl RequestUsageMut<'_> {
    fn mark_generation_output(&mut self) {
        if let Some(started) = self.generation_started_instant {
            *self.generation_finished_at = Some(SystemTime::now());
            *self.generation_duration = Some(started.elapsed());
        }
    }

    fn history_timestamp(&self) -> Option<SystemTime> {
        self.finished_at
            .map(|finished| self.generation_finished_at.unwrap_or(finished))
    }
}

#[derive(Debug, Clone)]
pub struct ActiveRequest {
    pub request_id: String,
    pub session_id: Option<String>,
    pub session_seq: Option<u64>,
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub endpoint: EndpointKind,
    pub started_at: SystemTime,
    started_instant: Instant,
    pub generation_started_at: Option<SystemTime>,
    generation_started_instant: Option<Instant>,
    generation_initial_output_tokens: u64,
    pub generation_finished_at: Option<SystemTime>,
    pub generation_duration: Option<Duration>,
    pub status: RequestStatus,
    pub streamed_bytes: u64,
    pub stream_chunks: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_detail: UsageDetail,
    pub error: Option<String>,
    pub traffic_capture_path: Option<PathBuf>,
}

impl ActiveRequest {
    pub fn elapsed(&self) -> Duration {
        self.started_instant.elapsed()
    }

    pub fn rate(&self) -> Throughput {
        throughput(
            self.output_tokens
                .and_then(|tokens| tokens.checked_sub(self.generation_initial_output_tokens)),
            self.streamed_bytes,
            self.stream_chunks,
            self.generation_duration.unwrap_or(Duration::ZERO),
        )
    }
}

#[derive(Debug, Clone)]
pub struct CompletedRequest {
    pub request_id: String,
    pub session_id: Option<String>,
    pub session_seq: Option<u64>,
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub endpoint: EndpointKind,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
    pub generation_started_at: Option<SystemTime>,
    generation_started_instant: Option<Instant>,
    generation_initial_output_tokens: u64,
    pub generation_finished_at: Option<SystemTime>,
    pub generation_duration: Option<Duration>,
    pub status: RequestStatus,
    pub http_status: Option<u16>,
    pub latency: Duration,
    pub streamed_bytes: u64,
    pub stream_chunks: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub usage_detail: UsageDetail,
    pub error: Option<String>,
    pub traffic_capture_path: Option<PathBuf>,
}

impl CompletedRequest {
    pub fn rate(&self) -> Throughput {
        throughput(
            self.output_tokens
                .and_then(|tokens| tokens.checked_sub(self.generation_initial_output_tokens)),
            self.streamed_bytes,
            self.stream_chunks,
            self.generation_duration.unwrap_or(Duration::ZERO),
        )
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum Throughput {
    TokensPerSecond(f64),
    BytesPerSecond(f64),
    EventsPerSecond(f64),
    None,
}

impl Throughput {
    pub fn label(&self) -> String {
        match self {
            Self::TokensPerSecond(value) => format!("{value:.1} tok/s"),
            Self::BytesPerSecond(value) if *value >= 1024.0 => {
                format!("{:.1} KB/s", value / 1024.0)
            }
            Self::BytesPerSecond(value) => format!("{value:.0} B/s"),
            Self::EventsPerSecond(value) => format!("{value:.1} ev/s"),
            Self::None => "-".to_string(),
        }
    }
}

/// Token counts in Anthropic semantics: `input` excludes cache reads and
/// writes. `None` means the field was not reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
    pub output: Option<u64>,
}

impl TokenUsage {
    /// Converts OpenAI usage, whose input total includes cached tokens.
    pub fn from_openai(input_total: Option<u64>, cached: Option<u64>, output: Option<u64>) -> Self {
        Self {
            input: input_total.map(|total| total.saturating_sub(cached.unwrap_or(0))),
            cache_read: input_total.map(|_| cached.unwrap_or(0)).or(cached),
            cache_write_5m: None,
            cache_write_1h: None,
            output,
        }
    }

    fn is_reported(&self) -> bool {
        self.input.is_some()
            || self.cache_read.is_some()
            || self.cache_write_5m.is_some()
            || self.cache_write_1h.is_some()
            || self.output.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostStatus {
    /// Priceable request without usage yet.
    Pending,
    Priced,
    /// Usage is known but no published price applies.
    Unpriced,
    /// The response could not be observed.
    Unmetered,
}

/// API-equivalent cost of one request at list prices.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestCost {
    pub status: CostStatus,
    #[serde(default)]
    pub micros: Option<u64>,
    #[serde(default)]
    pub estimated: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tier: PriceTier,
}

/// Summed API-equivalent cost in micro-USD.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CostTotals {
    pub micros: u64,
    #[serde(default)]
    pub unpriced: u32,
    #[serde(default)]
    pub unmetered: u32,
    #[serde(default)]
    pub estimated: u32,
    #[serde(default)]
    pub by_model: BTreeMap<String, u64>,
}

impl CostTotals {
    fn apply(&mut self, old: &CostContribution, new: &CostContribution) {
        self.micros = self
            .micros
            .saturating_sub(old.micros)
            .saturating_add(new.micros);
        self.unpriced = self
            .unpriced
            .saturating_sub(u32::from(old.unpriced))
            .saturating_add(u32::from(new.unpriced));
        self.unmetered = self
            .unmetered
            .saturating_sub(u32::from(old.unmetered))
            .saturating_add(u32::from(new.unmetered));
        self.estimated = self
            .estimated
            .saturating_sub(u32::from(old.estimated))
            .saturating_add(u32::from(new.estimated));
        if let Some(model) = &old.model
            && let Some(micros) = self.by_model.get_mut(model)
        {
            *micros = micros.saturating_sub(old.micros);
            if *micros == 0 {
                self.by_model.remove(model);
            }
        }
        if let Some(model) = &new.model
            && new.micros > 0
        {
            let micros = self.by_model.entry(model.clone()).or_default();
            *micros = micros.saturating_add(new.micros);
        }
    }
}

/// What one request currently adds to session and store-wide cost totals.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CostContribution {
    micros: u64,
    model: Option<String>,
    unpriced: bool,
    unmetered: bool,
    estimated: bool,
}

impl CostContribution {
    fn of(cost: Option<&RequestCost>) -> Self {
        let Some(cost) = cost else {
            return Self::default();
        };
        Self {
            micros: cost.micros.unwrap_or(0),
            model: cost.model.clone().filter(|_| cost.micros.is_some()),
            unpriced: cost.status == CostStatus::Unpriced,
            unmetered: cost.status == CostStatus::Unmetered,
            estimated: cost.estimated && cost.status == CostStatus::Priced,
        }
    }
}

/// Cache token detail and pricing state shared by active and completed
/// requests.
#[derive(Debug, Clone, Default)]
pub struct UsageDetail {
    pub cache_read_tokens: Option<u64>,
    pub cache_write_5m_tokens: Option<u64>,
    pub cache_write_1h_tokens: Option<u64>,
    /// `None` for requests outside the hardcoded price lists.
    pub cost: Option<RequestCost>,
    input_estimated: bool,
    cost_estimated: bool,
    unmetered: bool,
    selected_model: Option<String>,
    resolved_model: Option<String>,
    priced_model: Option<String>,
    price_tier: Option<PriceTier>,
    accounted: CostContribution,
}

impl UsageDetail {
    fn price(
        &mut self,
        provider: Option<&str>,
        endpoint: EndpointKind,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Option<(CostContribution, CostContribution)> {
        self.cost = self.compute_cost(provider, endpoint, input_tokens, output_tokens);
        let next = CostContribution::of(self.cost.as_ref());
        (next != self.accounted)
            .then(|| (std::mem::replace(&mut self.accounted, next.clone()), next))
    }

    fn compute_cost(
        &self,
        provider: Option<&str>,
        endpoint: EndpointKind,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Option<RequestCost> {
        if !matches!(
            endpoint,
            EndpointKind::Messages | EndpointKind::Responses | EndpointKind::ChatCompletions
        ) {
            return None;
        }
        let family = PriceFamily::for_provider(provider?)?;
        let requested = self
            .priced_model
            .as_deref()
            .or(self.resolved_model.as_deref())
            .or(self.selected_model.as_deref());
        let model = requested.map(|model| pricing::normalize_model(family, model));
        let tier = self
            .price_tier
            .or_else(|| {
                self.selected_model
                    .as_deref()
                    .and_then(pricing::tier_from_model_suffix)
            })
            .unwrap_or_default();
        let reported = input_tokens.is_some()
            || output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_write_5m_tokens.is_some()
            || self.cache_write_1h_tokens.is_some();
        let estimated = self.input_estimated || self.cost_estimated;
        let cost = |status, micros| RequestCost {
            status,
            micros,
            estimated,
            model: model.clone(),
            tier,
        };
        if !reported {
            let status = if self.unmetered {
                CostStatus::Unmetered
            } else {
                CostStatus::Pending
            };
            return Some(cost(status, None));
        }
        let usage = PricedUsage {
            input: input_tokens.unwrap_or(0),
            cache_read: self.cache_read_tokens.unwrap_or(0),
            cache_write_5m: self.cache_write_5m_tokens.unwrap_or(0),
            cache_write_1h: self.cache_write_1h_tokens.unwrap_or(0),
            output: output_tokens.unwrap_or(0),
        };
        Some(
            match model
                .as_deref()
                .and_then(|model| pricing::estimate_micros(family, model, tier, &usage))
            {
                Some(micros) => cost(CostStatus::Priced, Some(micros)),
                None => cost(CostStatus::Unpriced, None),
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub started_at: SystemTime,
    pub uptime: Duration,
    pub sessions: Vec<SessionSummary>,
    pub active: Vec<ActiveRequest>,
    pub recent: Vec<CompletedRequest>,
    pub cost: CostTotals,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub active_count: usize,
    pub request_count: usize,
    pub failure_count: usize,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub last_seen: SystemTime,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub output_token_samples: Vec<(SystemTime, u64)>,
    rate_output_tokens: u64,
    pub generation_duration: Duration,
    pub last_status: String,
    pub cost: CostTotals,
}

impl SessionSummary {
    pub fn rate(&self) -> Throughput {
        throughput(
            Some(self.rate_output_tokens).filter(|tokens| *tokens > 0),
            0,
            0,
            self.generation_duration,
        )
    }

    pub fn label(&self) -> String {
        self.session_id
            .clone()
            .unwrap_or_else(|| "no-session".to_string())
    }
}

#[derive(Debug)]
struct MonitorStore {
    started_at: SystemTime,
    started_instant: Instant,
    last_session_prune_instant: Instant,
    active: HashMap<String, ActiveRequest>,
    recent: VecDeque<CompletedRequest>,
    session_usage: HashMap<Option<String>, SessionUsage>,
    session_output_buckets: HashMap<Option<String>, Vec<(u64, u64)>>,
    recent_limit: usize,
    cost_totals: CostTotals,
}

#[derive(Debug)]
struct SessionUsage {
    input_tokens: u64,
    output_tokens: u64,
    cost: CostTotals,
    last_seen: SystemTime,
}

impl Default for SessionUsage {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            cost: CostTotals::default(),
            last_seen: SystemTime::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MonitorHandle {
    store: Arc<Mutex<MonitorStore>>,
}

impl Default for MonitorHandle {
    fn default() -> Self {
        Self::new(DEFAULT_RECENT_LIMIT)
    }
}

impl MonitorHandle {
    pub fn new(recent_limit: usize) -> Self {
        Self {
            store: Arc::new(Mutex::new(MonitorStore {
                started_at: SystemTime::now(),
                started_instant: Instant::now(),
                last_session_prune_instant: Instant::now(),
                active: HashMap::new(),
                recent: VecDeque::new(),
                session_usage: HashMap::new(),
                session_output_buckets: HashMap::new(),
                recent_limit,
                cost_totals: CostTotals::default(),
            })),
        }
    }

    pub fn publish(&self, event: MonitorEvent) {
        if let Ok(mut store) = self.store.lock() {
            store.apply(event);
        }
    }

    pub fn snapshot(&self) -> MonitorState {
        match self.store.lock() {
            Ok(mut store) => store.snapshot(),
            Err(_) => MonitorState {
                started_at: SystemTime::now(),
                uptime: Duration::ZERO,
                sessions: Vec::new(),
                active: Vec::new(),
                recent: Vec::new(),
                cost: CostTotals::default(),
            },
        }
    }

    pub fn request_started(
        &self,
        request_id: impl Into<String>,
        session_id: Option<String>,
        session_seq: Option<u64>,
        endpoint: EndpointKind,
    ) {
        self.publish(MonitorEvent::RequestStarted {
            request_id: request_id.into(),
            session_id,
            session_seq,
            endpoint,
        });
    }

    pub fn project_resolved(&self, request_id: impl Into<String>, project: impl Into<String>) {
        self.publish(MonitorEvent::ProjectResolved {
            request_id: request_id.into(),
            project: project.into(),
        });
    }

    pub fn session_sequence_resolved(&self, request_id: impl Into<String>, session_seq: u64) {
        self.publish(MonitorEvent::SessionSequenceResolved {
            request_id: request_id.into(),
            session_seq,
        });
    }

    pub fn provider_selected(
        &self,
        request_id: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        effort: Option<String>,
    ) {
        self.publish(MonitorEvent::ProviderSelected {
            request_id: request_id.into(),
            provider: provider.into(),
            model: model.into(),
            effort,
        });
    }

    pub fn model_resolved(&self, request_id: impl Into<String>, model: impl Into<String>) {
        self.publish(MonitorEvent::ModelResolved {
            request_id: request_id.into(),
            model: model.into(),
        });
    }

    pub fn compaction_started(&self, request_id: impl Into<String>) {
        self.publish(MonitorEvent::CompactionStarted {
            request_id: request_id.into(),
        });
    }

    pub fn upstream_started(&self, request_id: impl Into<String>) {
        self.publish(MonitorEvent::UpstreamStarted {
            request_id: request_id.into(),
        });
    }

    pub fn generation_started(&self, request_id: impl Into<String>) {
        self.publish(MonitorEvent::GenerationStarted {
            request_id: request_id.into(),
        });
    }

    pub fn traffic_capture_path(&self, request_id: impl Into<String>, path: PathBuf) {
        self.publish(MonitorEvent::TrafficCapturePath {
            request_id: request_id.into(),
            path,
        });
    }

    pub fn stream_progress(
        &self,
        request_id: impl Into<String>,
        bytes: u64,
        chunks: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        self.publish(MonitorEvent::StreamProgress {
            request_id: request_id.into(),
            bytes,
            chunks,
            input_tokens,
            output_tokens,
            input_estimated: false,
        });
    }

    /// Like [`Self::stream_progress`], but the input count is a local estimate
    /// that provider-reported final usage later replaces.
    pub fn stream_progress_estimated_input(
        &self,
        request_id: impl Into<String>,
        bytes: u64,
        chunks: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        self.publish(MonitorEvent::StreamProgress {
            request_id: request_id.into(),
            bytes,
            chunks,
            input_tokens,
            output_tokens,
            input_estimated: true,
        });
    }

    pub fn usage_observed(&self, request_id: impl Into<String>, usage: TokenUsage) {
        self.publish(MonitorEvent::UsageObserved {
            request_id: request_id.into(),
            usage,
        });
    }

    pub fn usage_final(&self, request_id: impl Into<String>, usage: TokenUsage) {
        self.publish(MonitorEvent::UsageFinal {
            request_id: request_id.into(),
            usage,
        });
    }

    pub fn pricing_resolved(
        &self,
        request_id: impl Into<String>,
        model: impl Into<String>,
        tier: PriceTier,
    ) {
        self.publish(MonitorEvent::PricingResolved {
            request_id: request_id.into(),
            model: model.into(),
            tier,
        });
    }

    pub fn cost_estimated(&self, request_id: impl Into<String>) {
        self.publish(MonitorEvent::CostEstimated {
            request_id: request_id.into(),
        });
    }

    pub fn usage_unmetered(&self, request_id: impl Into<String>) {
        self.publish(MonitorEvent::UsageUnmetered {
            request_id: request_id.into(),
        });
    }

    pub fn usage_updated(
        &self,
        request_id: impl Into<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        self.publish(MonitorEvent::UsageUpdated {
            request_id: request_id.into(),
            input_tokens,
            output_tokens,
        });
    }

    pub fn request_completed(
        &self,
        request_id: impl Into<String>,
        http_status: u16,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        self.publish(MonitorEvent::RequestCompleted {
            request_id: request_id.into(),
            http_status,
            input_tokens,
            output_tokens,
        });
    }

    pub fn request_failed(
        &self,
        request_id: impl Into<String>,
        http_status: Option<u16>,
        error: impl Into<String>,
    ) {
        self.publish(MonitorEvent::RequestFailed {
            request_id: request_id.into(),
            http_status,
            error: error.into(),
        });
    }

    pub fn request_abandoned(&self, request_id: impl Into<String>, error: impl Into<String>) {
        self.publish(MonitorEvent::RequestAbandoned {
            request_id: request_id.into(),
            error: error.into(),
        });
    }
}

impl MonitorStore {
    fn apply(&mut self, event: MonitorEvent) {
        let request_id = event.request_id().to_string();
        self.apply_event(event);
        self.reprice(&request_id);
        self.prune_session_history_if_due(SystemTime::now());
    }

    fn request_usage_mut(&mut self, request_id: &str) -> Option<RequestUsageMut<'_>> {
        if let Some(active) = self.active.get_mut(request_id) {
            return Some(RequestUsageMut {
                session_id: active.session_id.clone(),
                input_tokens: &mut active.input_tokens,
                output_tokens: &mut active.output_tokens,
                detail: &mut active.usage_detail,
                generation_started_instant: active.generation_started_instant,
                generation_finished_at: &mut active.generation_finished_at,
                generation_duration: &mut active.generation_duration,
                finished_at: None,
            });
        }
        self.recent
            .iter_mut()
            .find(|request| request.request_id == request_id)
            .map(|completed| RequestUsageMut {
                session_id: completed.session_id.clone(),
                input_tokens: &mut completed.input_tokens,
                output_tokens: &mut completed.output_tokens,
                detail: &mut completed.usage_detail,
                generation_started_instant: completed.generation_started_instant,
                generation_finished_at: &mut completed.generation_finished_at,
                generation_duration: &mut completed.generation_duration,
                finished_at: Some(completed.finished_at),
            })
    }

    fn reprice(&mut self, request_id: &str) {
        let update = if let Some(active) = self.active.get_mut(request_id) {
            active
                .usage_detail
                .price(
                    active.provider.as_deref(),
                    active.endpoint,
                    active.input_tokens,
                    active.output_tokens,
                )
                .map(|change| (active.session_id.clone(), change))
        } else if let Some(completed) = self
            .recent
            .iter_mut()
            .find(|request| request.request_id == request_id)
        {
            completed
                .usage_detail
                .price(
                    completed.provider.as_deref(),
                    completed.endpoint,
                    completed.input_tokens,
                    completed.output_tokens,
                )
                .map(|change| (completed.session_id.clone(), change))
        } else {
            None
        };
        if let Some((session_id, (old, new))) = update {
            self.cost_totals.apply(&old, &new);
            let usage = self.session_usage.entry(session_id).or_default();
            usage.cost.apply(&old, &new);
            usage.last_seen = SystemTime::now();
        }
    }

    fn apply_event(&mut self, event: MonitorEvent) {
        match event {
            MonitorEvent::RequestStarted {
                request_id,
                session_id,
                session_seq,
                endpoint,
            } => {
                self.active.insert(
                    request_id.clone(),
                    ActiveRequest {
                        request_id,
                        session_id,
                        session_seq,
                        project: None,
                        provider: None,
                        model: None,
                        effort: None,
                        endpoint,
                        started_at: SystemTime::now(),
                        started_instant: Instant::now(),
                        generation_started_at: None,
                        generation_started_instant: None,
                        generation_initial_output_tokens: 0,
                        generation_finished_at: None,
                        generation_duration: None,
                        status: RequestStatus::Started,
                        streamed_bytes: 0,
                        stream_chunks: 0,
                        input_tokens: None,
                        output_tokens: None,
                        usage_detail: UsageDetail::default(),
                        error: None,
                        traffic_capture_path: None,
                    },
                );
            }
            MonitorEvent::ProjectResolved {
                request_id,
                project,
            } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.project = Some(project);
                }
            }
            MonitorEvent::SessionSequenceResolved {
                request_id,
                session_seq,
            } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.session_seq = Some(session_seq);
                }
            }
            MonitorEvent::ProviderSelected {
                request_id,
                provider,
                model,
                effort,
            } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.provider = Some(provider);
                    active.usage_detail.selected_model = Some(model.clone());
                    active.model = Some(model);
                    active.effort = effort;
                    active.status = RequestStatus::ProviderSelected;
                }
            }
            MonitorEvent::ModelResolved { request_id, model } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.usage_detail.resolved_model = Some(model.clone());
                    active.model = Some(match active.model.take() {
                        Some(incoming) if incoming != model => format!("{incoming} → {model}"),
                        Some(incoming) => incoming,
                        None => model,
                    });
                }
            }
            MonitorEvent::CompactionStarted { request_id } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.status = RequestStatus::Compacting;
                }
            }
            MonitorEvent::UpstreamStarted { request_id } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.status = RequestStatus::Upstream;
                }
            }
            MonitorEvent::GenerationStarted { request_id } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.generation_started_at = Some(SystemTime::now());
                    active.generation_started_instant = Some(Instant::now());
                    active.generation_initial_output_tokens = active.output_tokens.unwrap_or(0);
                    active.generation_finished_at = None;
                    active.generation_duration = None;
                }
            }
            MonitorEvent::TrafficCapturePath { request_id, path } => {
                if let Some(active) = self.active.get_mut(&request_id) {
                    active.traffic_capture_path = Some(path);
                }
            }
            MonitorEvent::StreamProgress {
                request_id,
                bytes,
                chunks,
                input_tokens,
                output_tokens,
                input_estimated,
            } => {
                let mut usage_update = None;
                let mut history_update = None;
                if let Some(active) = self.active.get_mut(&request_id) {
                    if raises_token_count(&active.input_tokens, input_tokens) {
                        active.usage_detail.input_estimated = input_estimated;
                    }
                    active.status = RequestStatus::Streaming;
                    if active.generation_started_instant.is_none() {
                        active.generation_started_at = Some(SystemTime::now());
                        active.generation_started_instant = Some(Instant::now());
                        active.generation_initial_output_tokens =
                            output_tokens.or(active.output_tokens).unwrap_or(0);
                    } else {
                        active.generation_finished_at = Some(SystemTime::now());
                        active.generation_duration = active
                            .generation_started_instant
                            .map(|started| started.elapsed());
                    }
                    active.streamed_bytes = active.streamed_bytes.saturating_add(bytes);
                    active.stream_chunks = active.stream_chunks.saturating_add(chunks);
                    let input_delta = update_token_count(&mut active.input_tokens, input_tokens);
                    let output_delta = update_token_count(&mut active.output_tokens, output_tokens);
                    usage_update = Some((active.session_id.clone(), input_delta, output_delta));
                } else if let Some(completed) = self
                    .recent
                    .iter_mut()
                    .find(|request| request.request_id == request_id)
                {
                    if let Some(started) = completed.generation_started_instant {
                        completed.generation_finished_at = Some(SystemTime::now());
                        completed.generation_duration = Some(started.elapsed());
                    }
                    completed.streamed_bytes = completed.streamed_bytes.saturating_add(bytes);
                    completed.stream_chunks = completed.stream_chunks.saturating_add(chunks);
                    if raises_token_count(&completed.input_tokens, input_tokens) {
                        completed.usage_detail.input_estimated = input_estimated;
                    }
                    let input_delta = update_token_count(&mut completed.input_tokens, input_tokens);
                    let output_delta =
                        update_token_count(&mut completed.output_tokens, output_tokens);
                    usage_update = Some((completed.session_id.clone(), input_delta, output_delta));
                    if output_delta > 0 {
                        history_update = Some((
                            completed.session_id.clone(),
                            completed
                                .generation_finished_at
                                .unwrap_or(completed.finished_at),
                            output_delta,
                        ));
                    }
                }
                if let Some((session_id, input_delta, output_delta)) = usage_update {
                    self.record_session_usage(session_id, input_delta, output_delta);
                }
                if let Some((session_id, timestamp, tokens)) = history_update {
                    self.record_session_output(session_id, timestamp, tokens);
                }
            }
            MonitorEvent::UsageUpdated {
                request_id,
                input_tokens,
                output_tokens,
            } => {
                let mut usage_update = None;
                let mut history_update = None;
                if let Some(active) = self.active.get_mut(&request_id) {
                    if output_tokens.is_some()
                        && let Some(started) = active.generation_started_instant
                    {
                        active.generation_finished_at = Some(SystemTime::now());
                        active.generation_duration = Some(started.elapsed());
                    }
                    if raises_token_count(&active.input_tokens, input_tokens) {
                        active.usage_detail.input_estimated = false;
                    }
                    let input_delta = update_token_count(&mut active.input_tokens, input_tokens);
                    let output_delta = update_token_count(&mut active.output_tokens, output_tokens);
                    usage_update = Some((active.session_id.clone(), input_delta, output_delta));
                } else if let Some(completed) = self
                    .recent
                    .iter_mut()
                    .find(|request| request.request_id == request_id)
                {
                    if raises_token_count(&completed.input_tokens, input_tokens) {
                        completed.usage_detail.input_estimated = false;
                    }
                    if output_tokens.is_some()
                        && let Some(started) = completed.generation_started_instant
                    {
                        completed.generation_finished_at = Some(SystemTime::now());
                        completed.generation_duration = Some(started.elapsed());
                    }
                    let input_delta = update_token_count(&mut completed.input_tokens, input_tokens);
                    let output_delta =
                        update_token_count(&mut completed.output_tokens, output_tokens);
                    usage_update = Some((completed.session_id.clone(), input_delta, output_delta));
                    if output_delta > 0 {
                        history_update = Some((
                            completed.session_id.clone(),
                            completed
                                .generation_finished_at
                                .unwrap_or(completed.finished_at),
                            output_delta,
                        ));
                    }
                }
                if let Some((session_id, input_delta, output_delta)) = usage_update {
                    self.record_session_usage(session_id, input_delta, output_delta);
                }
                if let Some((session_id, timestamp, tokens)) = history_update {
                    self.record_session_output(session_id, timestamp, tokens);
                }
            }
            MonitorEvent::RequestCompleted {
                request_id,
                http_status,
                input_tokens,
                output_tokens,
            } => {
                self.finish(
                    &request_id,
                    RequestStatus::Completed,
                    Some(http_status),
                    input_tokens,
                    output_tokens,
                    None,
                );
            }
            MonitorEvent::RequestFailed {
                request_id,
                http_status,
                error,
            } => {
                self.finish(
                    &request_id,
                    RequestStatus::Failed,
                    http_status,
                    None,
                    None,
                    Some(error),
                );
            }
            MonitorEvent::RequestAbandoned { request_id, error } => {
                self.finish_active(
                    &request_id,
                    RequestStatus::Failed,
                    None,
                    None,
                    None,
                    Some(error),
                );
            }
            MonitorEvent::UsageObserved { request_id, usage } => {
                self.apply_usage(&request_id, usage, false);
            }
            MonitorEvent::UsageFinal { request_id, usage } => {
                self.apply_usage(&request_id, usage, true);
            }
            MonitorEvent::PricingResolved {
                request_id,
                model,
                tier,
            } => {
                if let Some(request) = self.request_usage_mut(&request_id) {
                    request.detail.priced_model = Some(model);
                    request.detail.price_tier = Some(tier);
                }
            }
            MonitorEvent::CostEstimated { request_id } => {
                if let Some(request) = self.request_usage_mut(&request_id) {
                    request.detail.cost_estimated = true;
                }
            }
            MonitorEvent::UsageUnmetered { request_id } => {
                if let Some(request) = self.request_usage_mut(&request_id) {
                    request.detail.unmetered = true;
                }
            }
        }
    }

    /// Progress observations only raise counts; final usage replaces every
    /// field it carries and corrects session totals by the signed difference.
    fn apply_usage(&mut self, request_id: &str, usage: TokenUsage, is_final: bool) {
        if !usage.is_reported() {
            return;
        }
        let Some(mut request) = self.request_usage_mut(request_id) else {
            return;
        };
        if usage.output.is_some() {
            request.mark_generation_output();
        }
        let (input_delta, output_delta) = if is_final {
            let input = replace_token_count(request.input_tokens, usage.input);
            if usage.input.is_some() {
                request.detail.input_estimated = false;
            }
            let output = replace_token_count(request.output_tokens, usage.output);
            for (field, value) in [
                (&mut request.detail.cache_read_tokens, usage.cache_read),
                (
                    &mut request.detail.cache_write_5m_tokens,
                    usage.cache_write_5m,
                ),
                (
                    &mut request.detail.cache_write_1h_tokens,
                    usage.cache_write_1h,
                ),
            ] {
                if value.is_some() {
                    *field = value;
                }
            }
            (input, output)
        } else {
            if raises_token_count(request.input_tokens, usage.input) {
                request.detail.input_estimated = false;
            }
            let input = update_token_count(request.input_tokens, usage.input) as i64;
            let output = update_token_count(request.output_tokens, usage.output) as i64;
            update_token_count(&mut request.detail.cache_read_tokens, usage.cache_read);
            update_token_count(
                &mut request.detail.cache_write_5m_tokens,
                usage.cache_write_5m,
            );
            update_token_count(
                &mut request.detail.cache_write_1h_tokens,
                usage.cache_write_1h,
            );
            (input, output)
        };
        let session_id = request.session_id.clone();
        let history = request
            .history_timestamp()
            .filter(|_| output_delta > 0)
            .map(|timestamp| (timestamp, output_delta as u64));
        self.record_session_usage_signed(session_id.clone(), input_delta, output_delta);
        if let Some((timestamp, tokens)) = history {
            self.record_session_output(session_id, timestamp, tokens);
        }
    }

    fn finish_active(
        &mut self,
        request_id: &str,
        status: RequestStatus,
        http_status: Option<u16>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        error: Option<String>,
    ) {
        if self.active.contains_key(request_id) {
            self.finish(
                request_id,
                status,
                http_status,
                input_tokens,
                output_tokens,
                error,
            );
        }
    }

    fn finish(
        &mut self,
        request_id: &str,
        status: RequestStatus,
        http_status: Option<u16>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        error: Option<String>,
    ) {
        let mut active = self
            .active
            .remove(request_id)
            .unwrap_or_else(|| ActiveRequest {
                request_id: request_id.to_string(),
                session_id: None,
                session_seq: None,
                project: None,
                provider: None,
                model: None,
                effort: None,
                endpoint: EndpointKind::Messages,
                started_at: SystemTime::now(),
                started_instant: Instant::now(),
                generation_started_at: None,
                generation_started_instant: None,
                generation_initial_output_tokens: 0,
                generation_finished_at: None,
                generation_duration: None,
                status: RequestStatus::Started,
                streamed_bytes: 0,
                stream_chunks: 0,
                input_tokens: None,
                output_tokens: None,
                usage_detail: UsageDetail::default(),
                error: None,
                traffic_capture_path: None,
            });
        if output_tokens.is_some()
            && let Some(started) = active.generation_started_instant
        {
            active.generation_finished_at = Some(SystemTime::now());
            active.generation_duration = Some(started.elapsed());
        }
        let input_delta = update_token_count(&mut active.input_tokens, input_tokens);
        let output_delta = update_token_count(&mut active.output_tokens, output_tokens);
        self.record_session_usage(active.session_id.clone(), input_delta, output_delta);
        let completed = CompletedRequest {
            request_id: active.request_id,
            session_id: active.session_id,
            session_seq: active.session_seq,
            project: active.project,
            provider: active.provider,
            model: active.model,
            effort: active.effort,
            endpoint: active.endpoint,
            started_at: active.started_at,
            finished_at: SystemTime::now(),
            generation_started_at: active.generation_started_at,
            generation_started_instant: active.generation_started_instant,
            generation_initial_output_tokens: active.generation_initial_output_tokens,
            generation_finished_at: active.generation_finished_at,
            generation_duration: active.generation_duration,
            status,
            http_status,
            latency: active.started_instant.elapsed(),
            streamed_bytes: active.streamed_bytes,
            stream_chunks: active.stream_chunks,
            input_tokens: active.input_tokens,
            output_tokens: active.output_tokens,
            usage_detail: active.usage_detail,
            error: error.or(active.error),
            traffic_capture_path: active.traffic_capture_path,
        };
        if let Some(tokens) = completed.output_tokens.filter(|tokens| *tokens > 0) {
            self.record_session_output(
                completed.session_id.clone(),
                completed
                    .generation_finished_at
                    .unwrap_or(completed.finished_at),
                tokens,
            );
        }
        self.recent.push_front(completed);
        while self.recent.len() > self.recent_limit {
            self.recent.pop_back();
        }
    }

    fn record_session_usage(
        &mut self,
        session_id: Option<String>,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        let usage = self.session_usage.entry(session_id).or_default();
        usage.input_tokens = usage.input_tokens.saturating_add(input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(output_tokens);
        usage.last_seen = SystemTime::now();
    }

    fn record_session_usage_signed(
        &mut self,
        session_id: Option<String>,
        input_delta: i64,
        output_delta: i64,
    ) {
        let usage = self.session_usage.entry(session_id).or_default();
        usage.input_tokens = usage.input_tokens.saturating_add_signed(input_delta);
        usage.output_tokens = usage.output_tokens.saturating_add_signed(output_delta);
        usage.last_seen = SystemTime::now();
    }

    fn record_session_output(
        &mut self,
        session_id: Option<String>,
        timestamp: SystemTime,
        tokens: u64,
    ) {
        let bucket = session_token_bucket(timestamp);
        let buckets = self.session_output_buckets.entry(session_id).or_default();
        match buckets.binary_search_by_key(&bucket, |(bucket, _)| *bucket) {
            Ok(index) => buckets[index].1 = buckets[index].1.saturating_add(tokens),
            Err(index) => buckets.insert(index, (bucket, tokens)),
        }
    }

    fn prune_session_history_if_due(&mut self, now: SystemTime) {
        if self.last_session_prune_instant.elapsed() >= SESSION_PRUNE_INTERVAL {
            self.prune_session_history(now);
        }
    }

    fn prune_session_history(&mut self, now: SystemTime) {
        let retained_sessions: HashSet<_> = self
            .active
            .values()
            .map(|request| request.session_id.clone())
            .chain(self.recent.iter().map(|request| request.session_id.clone()))
            .collect();
        self.session_usage.retain(|session_id, usage| {
            let retained_by_age = match now.duration_since(usage.last_seen) {
                Ok(age) => age < SESSION_USAGE_RETENTION,
                Err(_) => true,
            };
            retained_sessions.contains(session_id) || retained_by_age
        });
        let first_bucket = first_session_output_bucket(now);
        for buckets in self.session_output_buckets.values_mut() {
            buckets.retain(|(bucket, _)| *bucket >= first_bucket);
        }
        self.session_output_buckets
            .retain(|_, buckets| !buckets.is_empty());
        self.last_session_prune_instant = Instant::now();
    }

    fn snapshot(&mut self) -> MonitorState {
        let now = SystemTime::now();
        self.prune_session_history_if_due(now);
        let mut active: Vec<_> = self.active.values().cloned().collect();
        active.sort_by_key(|request| request.started_at);
        let sessions = session_summaries(
            &active,
            &self.recent,
            &self.session_usage,
            &self.session_output_buckets,
            now,
        );
        MonitorState {
            started_at: self.started_at,
            uptime: self.started_instant.elapsed(),
            sessions,
            active,
            recent: self.recent.iter().cloned().collect(),
            cost: self.cost_totals.clone(),
        }
    }
}

fn session_summaries(
    active: &[ActiveRequest],
    recent: &VecDeque<CompletedRequest>,
    session_usage: &HashMap<Option<String>, SessionUsage>,
    session_output_buckets: &HashMap<Option<String>, Vec<(u64, u64)>>,
    now: SystemTime,
) -> Vec<SessionSummary> {
    let mut sessions: HashMap<Option<String>, SessionSummary> = HashMap::new();
    for request in recent.iter().rev() {
        let entry = sessions
            .entry(request.session_id.clone())
            .or_insert_with(|| SessionSummary {
                session_id: request.session_id.clone(),
                project: request.project.clone(),
                active_count: 0,
                request_count: 0,
                failure_count: 0,
                provider: None,
                model: None,
                effort: None,
                last_seen: request.finished_at,
                input_tokens: 0,
                output_tokens: 0,
                output_token_samples: Vec::new(),
                rate_output_tokens: 0,
                generation_duration: Duration::ZERO,
                last_status: "-".to_string(),
                cost: CostTotals::default(),
            });
        entry.request_count += 1;
        if request.status == RequestStatus::Failed {
            entry.failure_count += 1;
        }
        entry.project = request.project.clone().or(entry.project.clone());
        entry.provider = request.provider.clone().or(entry.provider.clone());
        entry.model = request.model.clone().or(entry.model.clone());
        entry.effort = request.effort.clone().or(entry.effort.clone());
        entry.last_seen = max_system_time(entry.last_seen, request.finished_at);
        if let (Some(tokens), Some(duration)) = (
            request
                .output_tokens
                .and_then(|tokens| tokens.checked_sub(request.generation_initial_output_tokens))
                .filter(|tokens| *tokens > 0),
            request
                .generation_duration
                .filter(|duration| !duration.is_zero()),
        ) {
            entry.rate_output_tokens = entry.rate_output_tokens.saturating_add(tokens);
            entry.generation_duration = entry.generation_duration.saturating_add(duration);
        }
        entry.last_status = request.status.label().to_string();
    }

    for request in active {
        let entry = sessions
            .entry(request.session_id.clone())
            .or_insert_with(|| SessionSummary {
                session_id: request.session_id.clone(),
                project: request.project.clone(),
                active_count: 0,
                request_count: 0,
                failure_count: 0,
                provider: None,
                model: None,
                effort: None,
                last_seen: request.started_at,
                input_tokens: 0,
                output_tokens: 0,
                output_token_samples: Vec::new(),
                rate_output_tokens: 0,
                generation_duration: Duration::ZERO,
                last_status: "-".to_string(),
                cost: CostTotals::default(),
            });
        entry.active_count += 1;
        entry.request_count += 1;
        entry.project = request.project.clone().or(entry.project.clone());
        entry.provider = request.provider.clone().or(entry.provider.clone());
        entry.model = request.model.clone().or(entry.model.clone());
        entry.effort = request.effort.clone().or(entry.effort.clone());
        entry.last_seen = max_system_time(entry.last_seen, request.started_at);
        if let (Some(tokens), Some(duration)) = (
            request
                .output_tokens
                .and_then(|tokens| tokens.checked_sub(request.generation_initial_output_tokens))
                .filter(|tokens| *tokens > 0),
            request
                .generation_duration
                .filter(|duration| !duration.is_zero()),
        ) {
            entry.rate_output_tokens = entry.rate_output_tokens.saturating_add(tokens);
            entry.generation_duration = entry.generation_duration.saturating_add(duration);
        }
        entry.last_status = request.status.label().to_string();
    }

    let first_bucket = first_session_output_bucket(now);
    for (session_id, session) in &mut sessions {
        if let Some(usage) = session_usage.get(session_id) {
            session.input_tokens = usage.input_tokens;
            session.output_tokens = usage.output_tokens;
            session.cost = usage.cost.clone();
        }
        if let Some(buckets) = session_output_buckets.get(session_id) {
            session.output_token_samples = buckets
                .iter()
                .filter(|(bucket, _)| *bucket >= first_bucket)
                .map(|(bucket, tokens)| (session_token_bucket_start(*bucket), *tokens))
                .collect();
        }
    }

    let mut out: Vec<_> = sessions.into_values().collect();
    out.sort_by_key(SessionSummary::label);
    out
}

fn session_token_bucket(timestamp: SystemTime) -> u64 {
    timestamp
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        / SESSION_TOKEN_BUCKET_SECS
}

fn session_token_bucket_start(bucket: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(bucket.saturating_mul(SESSION_TOKEN_BUCKET_SECS))
}

fn first_session_output_bucket(now: SystemTime) -> u64 {
    session_token_bucket(now).saturating_sub(SESSION_OUTPUT_HISTORY_BUCKETS.saturating_sub(1))
}

fn update_token_count(current: &mut Option<u64>, incoming: Option<u64>) -> u64 {
    let Some(incoming) = incoming else {
        return 0;
    };
    let previous = current.unwrap_or(0);
    if incoming > previous || current.is_none() {
        *current = Some(incoming);
    }
    incoming.saturating_sub(previous)
}

fn raises_token_count(current: &Option<u64>, incoming: Option<u64>) -> bool {
    incoming.is_some_and(|incoming| current.is_none_or(|current| incoming > current))
}

/// Sets a provider-reported count and returns the signed change.
fn replace_token_count(current: &mut Option<u64>, incoming: Option<u64>) -> i64 {
    let Some(incoming) = incoming else {
        return 0;
    };
    let previous = current.replace(incoming).unwrap_or(0);
    i64::try_from(incoming)
        .unwrap_or(i64::MAX)
        .saturating_sub(i64::try_from(previous).unwrap_or(i64::MAX))
}

fn max_system_time(left: SystemTime, right: SystemTime) -> SystemTime {
    if right.duration_since(left).is_ok() {
        right
    } else {
        left
    }
}

pub fn throughput(
    output_tokens: Option<u64>,
    streamed_bytes: u64,
    stream_chunks: u64,
    elapsed: Duration,
) -> Throughput {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        return Throughput::None;
    }
    if let Some(tokens) = output_tokens.filter(|tokens| *tokens > 0) {
        return Throughput::TokensPerSecond(tokens as f64 / secs);
    }
    if streamed_bytes > 0 {
        return Throughput::BytesPerSecond(streamed_bytes as f64 / secs);
    }
    if stream_chunks > 0 {
        return Throughput::EventsPerSecond(stream_chunks as f64 / secs);
    }
    Throughput::None
}

pub fn usage_from_anthropic_sse(bytes: &[u8]) -> (Option<u64>, Option<u64>) {
    let text = String::from_utf8_lossy(bytes);
    let mut input_tokens = None;
    let mut output_tokens = None;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data.trim()) else {
            continue;
        };
        for usage in [
            value.pointer("/usage"),
            value.pointer("/delta/usage"),
            value.pointer("/message/usage"),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(tokens) = usage.get("input_tokens").and_then(|value| value.as_u64()) {
                input_tokens = Some(tokens);
            }
            if let Some(tokens) = usage.get("output_tokens").and_then(|value| value.as_u64()) {
                output_tokens = Some(tokens);
            }
        }
    }
    (input_tokens, output_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_requests_appear_active() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "r1",
            Some("s1".to_string()),
            Some(3),
            EndpointKind::Messages,
        );
        let state = monitor.snapshot();
        assert_eq!(state.active.len(), 1);
        assert_eq!(state.active[0].request_id, "r1");
        assert_eq!(state.active[0].session_id.as_deref(), Some("s1"));
        assert_eq!(state.active[0].session_seq, Some(3));
    }

    #[test]
    fn uptime_uses_the_service_monotonic_clock() {
        let monitor = MonitorHandle::new(10);
        {
            let mut store = monitor.store.lock().unwrap();
            store.started_at = SystemTime::now() + Duration::from_secs(3_600);
            store.started_instant = Instant::now() - Duration::from_secs(42);
        }

        let state = monitor.snapshot();

        assert!(state.started_at > SystemTime::now());
        assert!(state.uptime >= Duration::from_secs(42));
        assert!(state.uptime < Duration::from_secs(43));
    }

    #[test]
    fn resolved_model_appends_to_incoming_alias() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.provider_selected("r1", "codex", "claude-sonnet-4-6", None);
        monitor.model_resolved("r1", "gpt-5.4");

        let state = monitor.snapshot();
        assert_eq!(
            state.active[0].model.as_deref(),
            Some("claude-sonnet-4-6 → gpt-5.4")
        );
    }

    #[test]
    fn identical_resolved_model_is_shown_once() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.provider_selected("r1", "codex", "gpt-5.6-sol", None);
        monitor.model_resolved("r1", "gpt-5.6-sol");

        let state = monitor.snapshot();
        assert_eq!(state.active[0].model.as_deref(), Some("gpt-5.6-sol"));
    }

    #[test]
    fn compaction_started_marks_request_compacting() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.provider_selected("r1", "codex", "gpt-5.6-sol", None);
        monitor.compaction_started("r1");

        let state = monitor.snapshot();
        assert_eq!(state.active[0].status, RequestStatus::Compacting);
        assert_eq!(state.sessions[0].last_status, "compacting");
    }

    #[test]
    fn generation_baseline_pairs_total_usage_with_the_full_observed_interval() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.generation_started("r1");
        monitor.stream_progress("r1", 50, 1, Some(1_225), Some(141));
        monitor.request_completed("r1", 200, None, None);

        let request = &monitor.snapshot().recent[0];

        assert!(
            request
                .generation_duration
                .is_some_and(|duration| !duration.is_zero())
        );
        assert!(matches!(request.rate(), Throughput::TokensPerSecond(_)));
    }

    #[test]
    fn first_stream_progress_has_no_rate_without_an_interval() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.stream_progress("r1", 50, 1, Some(1_225), Some(141));

        let state = monitor.snapshot();
        assert_eq!(state.active.len(), 1);
        assert_eq!(state.active[0].rate(), Throughput::None);
    }

    #[test]
    fn late_stream_progress_extends_stream_timing_without_extending_request_latency() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.stream_progress("r1", 100, 1, Some(0), Some(0));
        monitor.request_completed("r1", 200, None, None);
        let completed = monitor.snapshot().recent[0].clone();
        monitor.stream_progress("r1", 50, 1, Some(1_225), Some(141));

        let state = monitor.snapshot();
        assert!(state.active.is_empty());
        assert_eq!(state.recent[0].streamed_bytes, 150);
        assert_eq!(state.recent[0].stream_chunks, 2);
        assert_eq!(state.recent[0].input_tokens, Some(1_225));
        assert_eq!(state.recent[0].output_tokens, Some(141));
        assert_eq!(state.recent[0].finished_at, completed.finished_at);
        assert_eq!(state.recent[0].latency, completed.latency);
        assert!(state.recent[0].generation_duration > completed.generation_duration);
        assert!(matches!(
            state.recent[0].rate(),
            Throughput::TokensPerSecond(_)
        ));
        assert_eq!(
            state.sessions[0]
                .output_token_samples
                .iter()
                .map(|(_, tokens)| *tokens)
                .sum::<u64>(),
            141
        );
    }

    #[test]
    fn completed_requests_leave_active_and_enter_recent() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.provider_selected("r1", "codex", "gpt-5.5", Some("high".to_string()));
        monitor.request_completed("r1", 200, Some(10), Some(20));
        let state = monitor.snapshot();
        assert!(state.active.is_empty());
        assert_eq!(state.recent.len(), 1);
        assert_eq!(state.recent[0].provider.as_deref(), Some("codex"));
        assert_eq!(state.recent[0].effort.as_deref(), Some("high"));
        assert_eq!(state.recent[0].output_tokens, Some(20));
    }

    #[test]
    fn failed_requests_preserve_error_summary() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.request_failed("r1", Some(400), "Unknown model");
        let state = monitor.snapshot();
        assert_eq!(state.recent[0].status, RequestStatus::Failed);
        assert_eq!(state.recent[0].http_status, Some(400));
        assert_eq!(state.recent[0].error.as_deref(), Some("Unknown model"));
    }

    #[test]
    fn abandoned_requests_leave_active_once() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.request_abandoned("r1", "request dropped");
        monitor.request_abandoned("r1", "request dropped again");
        let state = monitor.snapshot();
        assert!(state.active.is_empty());
        assert_eq!(state.recent.len(), 1);
        assert_eq!(state.recent[0].status, RequestStatus::Failed);
        assert_eq!(state.recent[0].http_status, None);
        assert_eq!(state.recent[0].error.as_deref(), Some("request dropped"));
    }

    #[test]
    fn completed_requests_ignore_late_abandonment() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", None, None, EndpointKind::Messages);
        monitor.request_completed("r1", 200, None, None);
        monitor.request_abandoned("r1", "request dropped");
        let state = monitor.snapshot();
        assert!(state.active.is_empty());
        assert_eq!(state.recent.len(), 1);
        assert_eq!(state.recent[0].status, RequestStatus::Completed);
    }

    #[test]
    fn bounded_recent_history_drops_oldest() {
        let monitor = MonitorHandle::new(2);
        for id in ["r1", "r2", "r3"] {
            monitor.request_started(id, None, None, EndpointKind::Messages);
            monitor.request_completed(id, 200, None, None);
        }
        let state = monitor.snapshot();
        let ids: Vec<_> = state
            .recent
            .iter()
            .map(|request| request.request_id.as_str())
            .collect();
        assert_eq!(ids, vec!["r3", "r2"]);
    }

    #[test]
    fn throughput_selects_best_available_signal() {
        let elapsed = Duration::from_secs(2);
        assert_eq!(
            throughput(Some(84), 1024, 10, elapsed),
            Throughput::TokensPerSecond(42.0)
        );
        assert_eq!(
            throughput(None, 2048, 10, elapsed),
            Throughput::BytesPerSecond(1024.0)
        );
        assert_eq!(
            throughput(None, 0, 36, elapsed),
            Throughput::EventsPerSecond(18.0)
        );
    }

    #[test]
    fn sse_usage_extracts_final_message_delta_tokens() {
        let sse = br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":0,"output_tokens":0}}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":12,"output_tokens":48}}

"#;
        assert_eq!(usage_from_anthropic_sse(sse), (Some(12), Some(48)));
    }

    fn completed_request(
        request_id: &str,
        session_id: &str,
        output_tokens: u64,
        latency: Duration,
        generation_duration: Option<Duration>,
    ) -> CompletedRequest {
        CompletedRequest {
            request_id: request_id.to_string(),
            session_id: Some(session_id.to_string()),
            session_seq: None,
            project: None,
            provider: Some("codex".to_string()),
            model: Some("gpt-5.6-sol".to_string()),
            effort: None,
            endpoint: EndpointKind::Messages,
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH + latency,
            generation_started_at: generation_duration.map(|_| SystemTime::UNIX_EPOCH),
            generation_started_instant: None,
            generation_initial_output_tokens: 0,
            generation_finished_at: generation_duration
                .map(|duration| SystemTime::UNIX_EPOCH + duration),
            generation_duration,
            status: RequestStatus::Completed,
            http_status: Some(200),
            latency,
            streamed_bytes: 0,
            stream_chunks: 0,
            input_tokens: None,
            output_tokens: Some(output_tokens),
            usage_detail: UsageDetail::default(),
            error: None,
            traffic_capture_path: None,
        }
    }

    fn session_summaries_for_requests(recent: &VecDeque<CompletedRequest>) -> Vec<SessionSummary> {
        let mut usage = HashMap::<Option<String>, SessionUsage>::new();
        for request in recent {
            let entry = usage.entry(request.session_id.clone()).or_default();
            entry.input_tokens = entry
                .input_tokens
                .saturating_add(request.input_tokens.unwrap_or(0));
            entry.output_tokens = entry
                .output_tokens
                .saturating_add(request.output_tokens.unwrap_or(0));
        }
        session_summaries(&[], recent, &usage, &HashMap::new(), SystemTime::now())
    }

    #[test]
    fn completed_request_rate_uses_stream_interval_instead_of_request_latency() {
        let request = completed_request(
            "r1",
            "s1",
            120,
            Duration::from_secs(30),
            Some(Duration::from_secs(4)),
        );

        assert_eq!(request.rate(), Throughput::TokensPerSecond(30.0));
    }

    #[test]
    fn request_rate_uses_token_delta_from_the_initial_observation() {
        let mut request = completed_request(
            "r1",
            "s1",
            120,
            Duration::from_secs(30),
            Some(Duration::from_secs(4)),
        );
        request.generation_initial_output_tokens = 20;

        assert_eq!(request.rate(), Throughput::TokensPerSecond(25.0));
    }

    #[test]
    fn session_rate_combines_request_tokens_and_generation_intervals() {
        let recent = VecDeque::from([
            completed_request(
                "r2",
                "s1",
                50,
                Duration::from_secs(40),
                Some(Duration::from_secs(1)),
            ),
            completed_request(
                "r1",
                "s1",
                100,
                Duration::from_secs(20),
                Some(Duration::from_secs(4)),
            ),
        ]);

        let sessions = session_summaries_for_requests(&recent);

        assert_eq!(sessions[0].output_tokens, 150);
        assert_eq!(sessions[0].generation_duration, Duration::from_secs(5));
        assert_eq!(sessions[0].rate(), Throughput::TokensPerSecond(30.0));
    }

    #[test]
    fn output_without_observed_stream_interval_has_no_output_rate() {
        let request = completed_request("r1", "s1", 120, Duration::from_secs(30), None);
        let recent = VecDeque::from([request.clone()]);

        assert_eq!(request.rate(), Throughput::None);
        assert_eq!(
            session_summaries_for_requests(&recent)[0].rate(),
            Throughput::None
        );
    }

    #[test]
    fn session_rate_excludes_interval_without_output_usage() {
        let mut tokenless = completed_request(
            "tokenless",
            "s1",
            0,
            Duration::from_secs(30),
            Some(Duration::from_secs(100)),
        );
        tokenless.output_tokens = None;
        let recent = VecDeque::from([
            tokenless,
            completed_request(
                "measured",
                "s1",
                100,
                Duration::from_secs(20),
                Some(Duration::from_secs(4)),
            ),
        ]);

        assert_eq!(
            session_summaries_for_requests(&recent)[0].rate(),
            Throughput::TokensPerSecond(25.0)
        );
    }

    #[test]
    fn session_rate_excludes_output_without_a_matching_stream_interval() {
        let recent = VecDeque::from([
            completed_request("buffered", "s1", 900, Duration::from_secs(30), None),
            completed_request(
                "streamed",
                "s1",
                100,
                Duration::from_secs(20),
                Some(Duration::from_secs(4)),
            ),
        ]);

        let session = &session_summaries_for_requests(&recent)[0];

        assert_eq!(session.output_tokens, 1_000);
        assert_eq!(session.rate(), Throughput::TokensPerSecond(25.0));
    }

    #[test]
    fn session_summaries_group_recent_and_active_requests() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "r1",
            Some("s1".to_string()),
            Some(1),
            EndpointKind::Messages,
        );
        monitor.project_resolved("r1", "example");
        monitor.provider_selected("r1", "codex", "gpt-5.5", None);
        monitor.request_completed("r1", 200, Some(10), Some(20));
        monitor.request_started(
            "r2",
            Some("s1".to_string()),
            Some(2),
            EndpointKind::Messages,
        );
        monitor.provider_selected("r2", "codex", "gpt-5.5", Some("xhigh".to_string()));
        let state = monitor.snapshot();
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].label(), "s1");
        assert_eq!(state.sessions[0].project.as_deref(), Some("example"));
        assert_eq!(state.sessions[0].request_count, 2);
        assert_eq!(state.sessions[0].active_count, 1);
        assert_eq!(state.sessions[0].effort.as_deref(), Some("xhigh"));
        assert_eq!(state.sessions[0].output_tokens, 20);
        assert_eq!(
            state.sessions[0]
                .output_token_samples
                .iter()
                .map(|(_, tokens)| *tokens)
                .collect::<Vec<_>>(),
            vec![20]
        );
    }

    #[test]
    fn session_output_history_survives_request_eviction() {
        let monitor = MonitorHandle::new(1);
        for (request_id, tokens) in [("oldest", 20), ("newest", 80)] {
            monitor.request_started(
                request_id,
                Some("s1".to_string()),
                None,
                EndpointKind::Messages,
            );
            monitor.request_completed(request_id, 200, Some(tokens * 10), Some(tokens));
        }

        let state = monitor.snapshot();

        assert_eq!(state.recent.len(), 1);
        assert_eq!(state.sessions[0].input_tokens, 1_000);
        assert_eq!(state.sessions[0].output_tokens, 100);
        assert_eq!(
            state.sessions[0]
                .output_token_samples
                .iter()
                .map(|(_, tokens)| *tokens)
                .sum::<u64>(),
            100
        );
    }

    #[test]
    fn session_output_history_prunes_old_buckets_without_changing_totals() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", Some("s1".to_string()), None, EndpointKind::Messages);
        monitor.request_completed("r1", 200, Some(100), Some(20));

        let now = SystemTime::now();
        let current_bucket = session_token_bucket(now);
        {
            let mut store = monitor.store.lock().unwrap();
            store
                .session_output_buckets
                .get_mut(&Some("s1".into()))
                .unwrap()
                .insert(
                    0,
                    (
                        current_bucket.saturating_sub(SESSION_OUTPUT_HISTORY_BUCKETS),
                        900,
                    ),
                );
            store.prune_session_history(now);
            assert_eq!(
                store
                    .session_output_buckets
                    .get(&Some("s1".into()))
                    .unwrap()
                    .len(),
                1
            );
        }

        let session = &monitor.snapshot().sessions[0];

        assert_eq!(session.input_tokens, 100);
        assert_eq!(session.output_tokens, 20);
        assert_eq!(
            session
                .output_token_samples
                .iter()
                .map(|(_, tokens)| *tokens)
                .sum::<u64>(),
            20
        );
        assert!(
            session
                .output_token_samples
                .iter()
                .all(|(timestamp, _)| session_token_bucket(*timestamp)
                    >= current_bucket.saturating_sub(SESSION_OUTPUT_HISTORY_BUCKETS - 1))
        );
    }

    #[test]
    fn snapshot_filters_old_buckets_before_prune_is_due() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started("r1", Some("s1".to_string()), None, EndpointKind::Messages);
        monitor.request_completed("r1", 200, Some(100), Some(20));

        let now = SystemTime::now();
        let current_bucket = session_token_bucket(now);
        let old_bucket = current_bucket.saturating_sub(SESSION_OUTPUT_HISTORY_BUCKETS);
        {
            let mut store = monitor.store.lock().unwrap();
            store
                .session_output_buckets
                .get_mut(&Some("s1".into()))
                .unwrap()
                .insert(0, (old_bucket, 900));
            store.last_session_prune_instant = Instant::now();
        }

        let session = &monitor.snapshot().sessions[0];

        assert_eq!(session.output_tokens, 20);
        assert_eq!(
            session
                .output_token_samples
                .iter()
                .map(|(_, tokens)| *tokens)
                .sum::<u64>(),
            20
        );
        let store = monitor.store.lock().unwrap();
        assert!(
            store
                .session_output_buckets
                .get(&Some("s1".into()))
                .unwrap()
                .iter()
                .any(|(bucket, tokens)| *bucket == old_bucket && *tokens == 900)
        );
    }

    #[test]
    fn inactive_session_accounting_expires_without_affecting_active_totals() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "active",
            Some("active-session".to_string()),
            None,
            EndpointKind::Messages,
        );
        monitor.usage_updated("active", Some(12), Some(7));
        monitor.request_started(
            "old",
            Some("old-session".to_string()),
            None,
            EndpointKind::Messages,
        );
        monitor.request_completed("old", 200, Some(30), Some(11));

        let old_timestamp = SystemTime::now() - SESSION_USAGE_RETENTION - Duration::from_secs(1);
        let old_bucket = session_token_bucket(old_timestamp);
        {
            let mut store = monitor.store.lock().unwrap();
            store.recent.clear();
            store
                .session_usage
                .get_mut(&Some("active-session".into()))
                .unwrap()
                .last_seen = old_timestamp;
            store
                .session_usage
                .get_mut(&Some("old-session".into()))
                .unwrap()
                .last_seen = old_timestamp;
            store
                .session_output_buckets
                .insert(Some("old-session".into()), vec![(old_bucket, 11)]);
            store.last_session_prune_instant = Instant::now() - SESSION_PRUNE_INTERVAL;
        }

        let state = monitor.snapshot();

        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].label(), "active-session");
        assert_eq!(state.sessions[0].input_tokens, 12);
        assert_eq!(state.sessions[0].output_tokens, 7);
        let store = monitor.store.lock().unwrap();
        assert!(
            !store
                .session_usage
                .contains_key(&Some("old-session".into()))
        );
        assert!(
            !store
                .session_output_buckets
                .contains_key(&Some("old-session".into()))
        );
    }

    #[test]
    fn session_usage_ignores_decreasing_request_observations() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "r1",
            Some("s1".to_string()),
            Some(1),
            EndpointKind::Messages,
        );
        monitor.usage_updated("r1", Some(100), Some(20));
        monitor.usage_updated("r1", Some(90), Some(15));
        monitor.usage_updated("r1", Some(120), Some(25));

        let active = monitor.snapshot();
        assert_eq!(active.active[0].input_tokens, Some(120));
        assert_eq!(active.active[0].output_tokens, Some(25));
        assert_eq!(active.sessions[0].input_tokens, 120);
        assert_eq!(active.sessions[0].output_tokens, 25);

        monitor.request_completed("r1", 200, Some(80), Some(10));
        let completed = monitor.snapshot();
        assert_eq!(completed.recent[0].input_tokens, Some(120));
        assert_eq!(completed.recent[0].output_tokens, Some(25));
        assert_eq!(completed.sessions[0].input_tokens, 120);
        assert_eq!(completed.sessions[0].output_tokens, 25);
    }

    #[test]
    fn compaction_preserves_cumulative_session_usage() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "before",
            Some("s1".to_string()),
            Some(1),
            EndpointKind::Messages,
        );
        monitor.request_completed("before", 200, Some(100), Some(20));
        monitor.request_started(
            "compact",
            Some("s1".to_string()),
            Some(2),
            EndpointKind::Messages,
        );
        monitor.compaction_started("compact");
        monitor.request_completed("compact", 200, Some(40), Some(10));

        let state = monitor.snapshot();
        assert_eq!(state.sessions[0].input_tokens, 140);
        assert_eq!(state.sessions[0].output_tokens, 30);
    }

    #[test]
    fn session_sequence_restart_preserves_cumulative_usage() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "before",
            Some("s1".to_string()),
            None,
            EndpointKind::Messages,
        );
        monitor.session_sequence_resolved("before", 7);
        monitor.request_completed("before", 200, Some(100), Some(20));

        monitor.request_started(
            "after",
            Some("s1".to_string()),
            None,
            EndpointKind::Messages,
        );
        monitor.session_sequence_resolved("after", 1);
        monitor.request_completed("after", 200, Some(25), Some(5));

        let state = monitor.snapshot();
        assert_eq!(state.sessions[0].input_tokens, 125);
        assert_eq!(state.sessions[0].output_tokens, 25);
    }

    #[test]
    fn session_order_is_stable_across_activity() {
        let monitor = MonitorHandle::new(10);
        monitor.request_started(
            "r1",
            Some("session-b".to_string()),
            Some(1),
            EndpointKind::Messages,
        );
        monitor.request_started(
            "r2",
            Some("session-a".to_string()),
            Some(1),
            EndpointKind::Messages,
        );

        let first: Vec<_> = monitor
            .snapshot()
            .sessions
            .iter()
            .map(SessionSummary::label)
            .collect();
        monitor.request_completed("r1", 200, None, None);
        monitor.request_started(
            "r3",
            Some("session-b".to_string()),
            Some(2),
            EndpointKind::Messages,
        );
        let second: Vec<_> = monitor
            .snapshot()
            .sessions
            .iter()
            .map(SessionSummary::label)
            .collect();

        assert_eq!(first, vec!["session-a", "session-b"]);
        assert_eq!(second, first);
    }

    fn priced_request(
        monitor: &MonitorHandle,
        request_id: &str,
        session_id: &str,
        provider: &str,
        model: &str,
    ) {
        monitor.request_started(
            request_id,
            Some(session_id.to_string()),
            None,
            EndpointKind::Messages,
        );
        monitor.provider_selected(request_id, provider, model, None);
        monitor.model_resolved(request_id, model);
    }

    fn final_usage(input: u64, output: u64) -> TokenUsage {
        TokenUsage {
            input: Some(input),
            output: Some(output),
            ..TokenUsage::default()
        }
    }

    const MTOK: u64 = 1_000_000;

    #[test]
    fn session_and_store_cost_survive_recent_eviction() {
        let monitor = MonitorHandle::new(2);
        for index in 0..5 {
            let id = format!("r{index}");
            priced_request(&monitor, &id, "s", "codex", "gpt-6-astra");
            monitor.usage_final(&id, final_usage(100_000, 0));
            monitor.request_completed(&id, 200, None, None);
        }
        let state = monitor.snapshot();
        assert_eq!(state.recent.len(), 2);
        // Five requests of 100K Astra input at $10 per MTok.
        assert_eq!(state.sessions[0].cost.micros, 5 * MTOK);
        assert_eq!(state.cost.micros, 5 * MTOK);
        assert_eq!(state.cost.by_model.get("gpt-6-astra"), Some(&(5 * MTOK)));
    }

    #[test]
    fn count_tokens_and_unpriced_providers_add_no_cost() {
        let monitor = MonitorHandle::default();
        monitor.request_started("count", Some("s".into()), None, EndpointKind::CountTokens);
        monitor.provider_selected("count", "anthropic", "claude-opus-5-5", None);
        monitor.request_completed("count", 200, Some(MTOK), None);
        priced_request(&monitor, "kimi", "s", "kimi", "kimi-k2.6");
        monitor.usage_updated("kimi", Some(MTOK), Some(MTOK));
        monitor.request_completed("kimi", 200, None, None);
        let state = monitor.snapshot();
        assert!(
            state
                .recent
                .iter()
                .all(|request| request.usage_detail.cost.is_none())
        );
        assert_eq!(state.cost, CostTotals::default());
    }

    #[test]
    fn mixed_claude_and_codex_session_breaks_cost_down_by_model() {
        let monitor = MonitorHandle::default();
        priced_request(&monitor, "claude", "dual", "anthropic", "claude-opus-5-5");
        monitor.usage_final(
            "claude",
            TokenUsage {
                input: Some(MTOK),
                cache_read: Some(MTOK),
                cache_write_5m: Some(MTOK),
                cache_write_1h: Some(MTOK),
                output: Some(MTOK),
            },
        );
        monitor.request_completed("claude", 200, None, None);
        priced_request(&monitor, "codex", "dual", "codex", "gpt-6-astra");
        monitor.usage_final(
            "codex",
            TokenUsage::from_openai(Some(200_000), Some(100_000), Some(0)),
        );
        monitor.request_completed("codex", 200, None, None);
        let state = monitor.snapshot();
        // Opus 5.5: $4 + $0.20 + $5 + $8 + $20; Astra: $1 + $0.10 cached.
        let claude = 37_200_000;
        let astra = 1_100_000;
        let session = &state.sessions[0].cost;
        assert_eq!(session.by_model.get("claude-opus-5-5"), Some(&claude));
        assert_eq!(session.by_model.get("gpt-6-astra"), Some(&astra));
        assert_eq!(session.micros, claude + astra);
        assert_eq!(state.cost.micros, claude + astra);
    }

    #[test]
    fn pricing_key_change_moves_cost_and_clears_unpriced() {
        let monitor = MonitorHandle::default();
        priced_request(&monitor, "r", "s", "codex", "gpt-5.3-codex-spark");
        monitor.usage_final("r", final_usage(100_000, 0));
        let state = monitor.snapshot();
        assert_eq!(state.cost.unpriced, 1);
        assert_eq!(state.cost.micros, 0);
        assert_eq!(
            state.active[0]
                .usage_detail
                .cost
                .as_ref()
                .map(|cost| cost.status),
            Some(CostStatus::Unpriced)
        );
        monitor.pricing_resolved("r", "gpt-6-sol", PriceTier::Standard);
        let state = monitor.snapshot();
        assert_eq!(state.cost.unpriced, 0);
        assert_eq!(state.cost.by_model.get("gpt-6-sol"), Some(&200_000));
        monitor.pricing_resolved("r", "gpt-6-astra", PriceTier::Fast);
        let state = monitor.snapshot();
        assert_eq!(state.cost.by_model.get("gpt-6-sol"), None);
        assert_eq!(state.cost.by_model.get("gpt-6-astra"), Some(&(2 * MTOK)));
        assert_eq!(state.sessions[0].cost.micros, 2 * MTOK);
    }

    #[test]
    fn final_usage_after_completion_updates_recent_entry_and_totals() {
        let monitor = MonitorHandle::default();
        priced_request(&monitor, "r", "s", "anthropic", "claude-sonnet-5-5");
        monitor.request_completed("r", 200, None, None);
        let state = monitor.snapshot();
        assert_eq!(
            state.recent[0]
                .usage_detail
                .cost
                .as_ref()
                .map(|cost| cost.status),
            Some(CostStatus::Pending)
        );
        monitor.usage_final("r", final_usage(MTOK, MTOK));
        let state = monitor.snapshot();
        assert_eq!(state.recent[0].input_tokens, Some(MTOK));
        assert_eq!(
            state.recent[0]
                .usage_detail
                .cost
                .as_ref()
                .and_then(|cost| cost.micros),
            Some(12 * MTOK)
        );
        assert_eq!(state.sessions[0].input_tokens, MTOK);
        assert_eq!(state.cost.micros, 12 * MTOK);
    }

    #[test]
    fn final_usage_corrects_estimates_while_observations_only_raise() {
        let monitor = MonitorHandle::default();
        priced_request(&monitor, "r", "s", "codex", "gpt-6-luna");
        monitor.stream_progress_estimated_input("r", 10, 1, Some(5_000), Some(0));
        monitor.usage_observed("r", final_usage(4_000, 10));
        let state = monitor.snapshot();
        assert_eq!(state.active[0].input_tokens, Some(5_000));
        assert!(
            state.active[0]
                .usage_detail
                .cost
                .as_ref()
                .unwrap()
                .estimated
        );
        monitor.usage_final("r", final_usage(4_000, 10));
        let state = monitor.snapshot();
        assert_eq!(state.active[0].input_tokens, Some(4_000));
        assert!(
            !state.active[0]
                .usage_detail
                .cost
                .as_ref()
                .unwrap()
                .estimated
        );
        assert_eq!(state.sessions[0].input_tokens, 4_000);
        assert_eq!(state.cost.estimated, 0);
    }

    #[test]
    fn unmetered_and_estimated_requests_are_counted_separately() {
        let monitor = MonitorHandle::default();
        priced_request(&monitor, "opaque", "s", "anthropic", "claude-opus-5-5");
        monitor.usage_unmetered("opaque");
        monitor.request_completed("opaque", 200, None, None);
        priced_request(&monitor, "search", "s", "codex", "gpt-6-sol");
        monitor.usage_updated("search", Some(1_000), Some(100));
        monitor.cost_estimated("search");
        monitor.request_completed("search", 200, None, None);
        let state = monitor.snapshot();
        assert_eq!(state.cost.unmetered, 1);
        assert_eq!(state.cost.estimated, 1);
        assert_eq!(state.cost.unpriced, 0);
        assert_eq!(state.cost.micros, 3_000);
    }

    #[test]
    fn fast_tier_from_model_suffix_prices_without_pricing_event() {
        let monitor = MonitorHandle::default();
        monitor.request_started("r", None, None, EndpointKind::Messages);
        monitor.provider_selected("r", "codex", "gpt-6-astra-fast", None);
        monitor.usage_final("r", final_usage(100_000, 0));
        let state = monitor.snapshot();
        let cost = state.active[0].usage_detail.cost.clone().unwrap();
        assert_eq!(cost.tier, PriceTier::Fast);
        assert_eq!(cost.micros, Some(2 * MTOK));
    }
}
