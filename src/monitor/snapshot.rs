//! Immutable dashboard read models. Process-local clocks and mutable accounting
//! stay in the monitor store; viewers receive elapsed durations and computed rates.
use super::{
    ActiveRequest, CompletedRequest, CostTotals, EndpointKind, MonitorState, RequestCost,
    RequestStatus, SessionSummary, Throughput,
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub const PROTOCOL_VERSION: u32 = 1;

fn default_snapshot_at() -> SystemTime {
    SystemTime::now()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorResponse {
    pub version: u32,
    pub snapshot: MonitorSnapshot,
}

impl From<MonitorState> for MonitorResponse {
    fn from(state: MonitorState) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            snapshot: state.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SnapshotUpdate {
    Live(MonitorSnapshot),
    Disconnected {
        snapshot: MonitorSnapshot,
        error: String,
    },
}

impl SnapshotUpdate {
    pub fn snapshot(&self) -> &MonitorSnapshot {
        match self {
            Self::Live(snapshot) | Self::Disconnected { snapshot, .. } => snapshot,
        }
    }

    pub fn connection_error(&self) -> Option<&str> {
        match self {
            Self::Live(_) => None,
            Self::Disconnected { error, .. } => Some(error),
        }
    }

    pub(super) fn updated(self, result: Result<MonitorSnapshot, String>) -> Self {
        match result {
            Ok(snapshot) => Self::Live(snapshot),
            Err(error) => {
                let snapshot = match self {
                    Self::Live(snapshot) | Self::Disconnected { snapshot, .. } => snapshot,
                };
                Self::Disconnected { snapshot, error }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub started_at: SystemTime,
    #[serde(default = "default_snapshot_at")]
    pub snapshot_at: SystemTime,
    pub uptime: Duration,
    pub sessions: Vec<SessionSnapshot>,
    pub active: Vec<ActiveSnapshot>,
    pub recent: Vec<CompletedSnapshot>,
    /// API-equivalent cost since the proxy started; absent from older proxies.
    #[serde(default)]
    pub cost: Option<CostTotals>,
    #[serde(default)]
    pub prices_as_of: Option<String>,
}

impl From<MonitorState> for MonitorSnapshot {
    fn from(state: MonitorState) -> Self {
        Self {
            started_at: state.started_at,
            snapshot_at: SystemTime::now(),
            uptime: state.uptime,
            sessions: state.sessions.into_iter().map(Into::into).collect(),
            active: state.active.into_iter().map(Into::into).collect(),
            recent: state.recent.into_iter().map(Into::into).collect(),
            cost: Some(state.cost),
            prices_as_of: Some(crate::pricing::PRICES_AS_OF.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveSnapshot {
    pub request_id: String,
    pub session_id: Option<String>,
    pub session_seq: Option<u64>,
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub endpoint: EndpointKind,
    pub started_at: SystemTime,
    pub generation_started_at: Option<SystemTime>,
    pub generation_finished_at: Option<SystemTime>,
    pub generation_duration: Option<Duration>,
    pub status: RequestStatus,
    pub streamed_bytes: u64,
    pub stream_chunks: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_5m_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_1h_tokens: Option<u64>,
    #[serde(default)]
    pub cost: Option<RequestCost>,
    pub error: Option<String>,
    pub traffic_capture_path: Option<PathBuf>,
    elapsed: Duration,
    throughput: Throughput,
}

impl ActiveSnapshot {
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
    pub fn rate(&self) -> Throughput {
        self.throughput.clone()
    }
}

impl From<ActiveRequest> for ActiveSnapshot {
    fn from(value: ActiveRequest) -> Self {
        let throughput = value.rate();
        let elapsed = value.elapsed();
        Self {
            request_id: value.request_id,
            session_id: value.session_id,
            session_seq: value.session_seq,
            project: value.project,
            provider: value.provider,
            model: value.model,
            effort: value.effort,
            endpoint: value.endpoint,
            started_at: value.started_at,
            generation_started_at: value.generation_started_at,
            generation_finished_at: value.generation_finished_at,
            generation_duration: value.generation_duration,
            status: value.status,
            streamed_bytes: value.streamed_bytes,
            stream_chunks: value.stream_chunks,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cache_read_tokens: value.usage_detail.cache_read_tokens,
            cache_write_5m_tokens: value.usage_detail.cache_write_5m_tokens,
            cache_write_1h_tokens: value.usage_detail.cache_write_1h_tokens,
            cost: value.usage_detail.cost,
            error: value.error,
            traffic_capture_path: value.traffic_capture_path,
            elapsed,
            throughput,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletedSnapshot {
    pub request_id: String,
    pub session_id: Option<String>,
    pub session_seq: Option<u64>,
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub endpoint: EndpointKind,
    pub started_at: SystemTime,
    pub generation_started_at: Option<SystemTime>,
    pub generation_finished_at: Option<SystemTime>,
    pub generation_duration: Option<Duration>,
    pub status: RequestStatus,
    pub streamed_bytes: u64,
    pub stream_chunks: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_5m_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_1h_tokens: Option<u64>,
    #[serde(default)]
    pub cost: Option<RequestCost>,
    pub error: Option<String>,
    pub traffic_capture_path: Option<PathBuf>,
    pub finished_at: SystemTime,
    pub http_status: Option<u16>,
    pub latency: Duration,
    throughput: Throughput,
}

impl CompletedSnapshot {
    pub fn rate(&self) -> Throughput {
        self.throughput.clone()
    }
}

impl From<CompletedRequest> for CompletedSnapshot {
    fn from(value: CompletedRequest) -> Self {
        let throughput = value.rate();
        Self {
            request_id: value.request_id,
            session_id: value.session_id,
            session_seq: value.session_seq,
            project: value.project,
            provider: value.provider,
            model: value.model,
            effort: value.effort,
            endpoint: value.endpoint,
            started_at: value.started_at,
            generation_started_at: value.generation_started_at,
            generation_finished_at: value.generation_finished_at,
            generation_duration: value.generation_duration,
            status: value.status,
            streamed_bytes: value.streamed_bytes,
            stream_chunks: value.stream_chunks,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cache_read_tokens: value.usage_detail.cache_read_tokens,
            cache_write_5m_tokens: value.usage_detail.cache_write_5m_tokens,
            cache_write_1h_tokens: value.usage_detail.cache_write_1h_tokens,
            cost: value.usage_detail.cost,
            error: value.error,
            traffic_capture_path: value.traffic_capture_path,
            finished_at: value.finished_at,
            http_status: value.http_status,
            latency: value.latency,
            throughput,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
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
    pub generation_duration: Duration,
    pub last_status: String,
    #[serde(default)]
    pub cost: Option<CostTotals>,
    throughput: Throughput,
}

impl SessionSnapshot {
    pub fn rate(&self) -> Throughput {
        self.throughput.clone()
    }
}

impl From<SessionSummary> for SessionSnapshot {
    fn from(value: SessionSummary) -> Self {
        let throughput = value.rate();
        Self {
            session_id: value.session_id,
            project: value.project,
            active_count: value.active_count,
            request_count: value.request_count,
            failure_count: value.failure_count,
            provider: value.provider,
            model: value.model,
            effort: value.effort,
            last_seen: value.last_seen,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            output_token_samples: value.output_token_samples,
            generation_duration: value.generation_duration,
            last_status: value.last_status,
            cost: Some(value.cost),
            throughput,
        }
    }
}

impl SessionSnapshot {
    pub fn label(&self) -> String {
        self.session_id
            .clone()
            .unwrap_or_else(|| "no-session".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::MonitorHandle;

    #[test]
    fn snapshot_round_trip_preserves_accounting_without_process_clocks() {
        let monitor = MonitorHandle::default();
        monitor.request_started(
            "active",
            Some("session".into()),
            Some(2),
            EndpointKind::Messages,
        );
        monitor.provider_selected("active", "codex", "gpt-6-astra", Some("high".into()));
        monitor.stream_progress("active", 40, 1, Some(8), Some(3));
        monitor.request_started(
            "done",
            Some("session".into()),
            Some(1),
            EndpointKind::CountTokens,
        );
        monitor.request_completed("done", 200, Some(11), None);
        let snapshot = MonitorSnapshot::from(monitor.snapshot());
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("instant"));
        let decoded: MonitorSnapshot = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.snapshot_at, snapshot.snapshot_at);
        assert_eq!(decoded.uptime, snapshot.uptime);
        assert_eq!(decoded.active[0].elapsed(), snapshot.active[0].elapsed());
        assert_eq!(decoded.active[0].rate(), snapshot.active[0].rate());
        assert_eq!(decoded.sessions[0].rate(), snapshot.sessions[0].rate());
        assert_eq!(decoded.recent[0].http_status, Some(200));
    }

    #[test]
    fn snapshot_deserialization_accepts_legacy_and_newer_fields() {
        let snapshot: MonitorSnapshot = MonitorHandle::default().snapshot().into();
        let mut legacy = serde_json::to_value(&snapshot).unwrap();
        legacy.as_object_mut().unwrap().remove("snapshot_at");
        legacy.as_object_mut().unwrap().remove("cost");
        legacy.as_object_mut().unwrap().remove("prices_as_of");
        let decoded: MonitorSnapshot = serde_json::from_value(legacy).unwrap();
        assert!(decoded.snapshot_at >= snapshot.started_at);
        assert_eq!(decoded.cost, None);
        assert_eq!(decoded.prices_as_of, None);

        let mut newer = serde_json::to_value(&snapshot).unwrap();
        newer
            .as_object_mut()
            .unwrap()
            .insert("future_field".into(), serde_json::json!(true));
        let decoded: MonitorSnapshot = serde_json::from_value(newer).unwrap();
        assert_eq!(decoded.snapshot_at, snapshot.snapshot_at);
    }
}

#[cfg(test)]
mod cost_tests {
    use super::*;
    use crate::monitor::{CostStatus, MonitorHandle, TokenUsage};

    #[test]
    fn request_and_session_snapshots_without_cost_fields_still_decode() {
        let monitor = MonitorHandle::default();
        monitor.request_started("done", Some("s".into()), Some(1), EndpointKind::Messages);
        monitor.provider_selected("done", "anthropic", "claude-opus-5-5", None);
        monitor.usage_final(
            "done",
            TokenUsage {
                input: Some(10),
                cache_read: Some(1_000),
                output: Some(5),
                ..TokenUsage::default()
            },
        );
        monitor.request_completed("done", 200, None, None);
        let snapshot = MonitorSnapshot::from(monitor.snapshot());
        assert_eq!(
            snapshot.recent[0].cost.as_ref().map(|cost| cost.status),
            Some(CostStatus::Priced)
        );
        let mut legacy = serde_json::to_value(&snapshot).unwrap();
        for field in [
            "cache_read_tokens",
            "cache_write_5m_tokens",
            "cache_write_1h_tokens",
            "cost",
        ] {
            legacy["recent"][0].as_object_mut().unwrap().remove(field);
        }
        legacy["sessions"][0]
            .as_object_mut()
            .unwrap()
            .remove("cost");
        let decoded: MonitorSnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.recent[0].cost, None);
        assert_eq!(decoded.recent[0].cache_read_tokens, None);
        assert_eq!(decoded.sessions[0].cost, None);
    }
}
