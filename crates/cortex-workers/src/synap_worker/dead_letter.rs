//! Phase14h §1.3 — dead-letter sink + fixed reason taxonomy.
//!
//! Every worker that drops or fails to handle an envelope routes
//! the victim through this sink:
//!
//! - bumps the per-`(worker, reason)` counter on the shared
//!   [`super::metrics::WorkerMetrics`];
//! - emits a structured WARN log carrying the offset + reason +
//!   the originating event_id when available;
//! - hands the payload to the active [`DeadLetterSink`] which
//!   may persist it to the `cortex.dead_letter` Synap stream or
//!   any test fixture.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

/// Fixed taxonomy of reasons a worker drops an envelope. New
/// reasons require a code change so the doctor's per-reason
/// rendering stays stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterReason {
    /// Envelope failed JSON deserialisation.
    DeserializeFailed,
    /// Handler returned a permanent error (no retry budget).
    PermanentHandlerError,
    /// Retry budget exhausted on transient errors.
    RetryBudgetExhausted,
    /// Publishing the enriched output failed.
    PublishFailed,
    /// Envelope lacked a required field (e.g. event_id).
    MissingRequiredField,
}

impl DeadLetterReason {
    /// Wire label for the metric counter + JSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DeserializeFailed => "deserialize_failed",
            Self::PermanentHandlerError => "permanent_handler_error",
            Self::RetryBudgetExhausted => "retry_budget_exhausted",
            Self::PublishFailed => "publish_failed",
            Self::MissingRequiredField => "missing_required_field",
        }
    }
}

/// One dead-lettered envelope ready for the sink.
#[derive(Debug, Clone)]
pub struct DeadLetterRecord {
    /// Originating worker name (`embedder` / `fulltext` /
    /// `graph` / `classifier`).
    pub worker: String,
    /// Synap room the envelope came from.
    pub room: String,
    /// Synap offset of the original envelope.
    pub offset: u64,
    /// Reason the envelope was dropped.
    pub reason: DeadLetterReason,
    /// Best-effort error string (one line, no newlines).
    pub error: String,
    /// Original envelope (raw JSON). May be a truncated
    /// fragment when the source could not be deserialised.
    pub payload: Value,
}

/// Sink that records a dead-letter record.
#[async_trait]
pub trait DeadLetterSink: Send + Sync {
    /// Record one dead-letter envelope. Never returns Err — the
    /// sink swallows transport failures and surfaces them on
    /// its own (typically by logging at ERROR).
    async fn record(&self, record: DeadLetterRecord);
}

/// Test sink that buffers records in memory. Tests grab the
/// buffer via [`MemoryDeadLetterSink::drain`].
#[derive(Debug, Default)]
pub struct MemoryDeadLetterSink {
    buffer: Mutex<Vec<DeadLetterRecord>>,
}

impl MemoryDeadLetterSink {
    /// Fresh empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain every buffered record. Order matches insertion.
    pub fn drain(&self) -> Vec<DeadLetterRecord> {
        match self.buffer.lock() {
            Ok(mut g) => g.drain(..).collect(),
            Err(p) => p.into_inner().drain(..).collect(),
        }
    }

    /// Length of the buffer without draining.
    pub fn len(&self) -> usize {
        match self.buffer.lock() {
            Ok(g) => g.len(),
            Err(p) => p.into_inner().len(),
        }
    }

    /// `true` when the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl DeadLetterSink for MemoryDeadLetterSink {
    async fn record(&self, record: DeadLetterRecord) {
        let mut guard = match self.buffer.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.push(record);
    }
}

/// Convenience: wrap [`MemoryDeadLetterSink`] in an `Arc<dyn _>`
/// for callers that hold the trait object.
pub fn memory_sink() -> Arc<dyn DeadLetterSink> {
    Arc::new(MemoryDeadLetterSink::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reason_labels_are_stable() {
        assert_eq!(
            DeadLetterReason::DeserializeFailed.as_str(),
            "deserialize_failed"
        );
        assert_eq!(
            DeadLetterReason::PermanentHandlerError.as_str(),
            "permanent_handler_error"
        );
        assert_eq!(
            DeadLetterReason::RetryBudgetExhausted.as_str(),
            "retry_budget_exhausted"
        );
        assert_eq!(DeadLetterReason::PublishFailed.as_str(), "publish_failed");
        assert_eq!(
            DeadLetterReason::MissingRequiredField.as_str(),
            "missing_required_field"
        );
    }

    #[tokio::test]
    async fn memory_sink_buffers_and_drains() {
        let sink = MemoryDeadLetterSink::new();
        assert!(sink.is_empty());
        sink.record(DeadLetterRecord {
            worker: "embedder".into(),
            room: "cortex.events.enriched".into(),
            offset: 42,
            reason: DeadLetterReason::DeserializeFailed,
            error: "missing field `id`".into(),
            payload: json!({"raw": "data"}),
        })
        .await;
        assert_eq!(sink.len(), 1);
        let drained = sink.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].offset, 42);
        assert_eq!(drained[0].reason, DeadLetterReason::DeserializeFailed);
        assert!(sink.is_empty());
    }
}
