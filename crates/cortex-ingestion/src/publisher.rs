//! Synap publisher abstraction. Uses `synap-sdk` under the hood.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex;
use synap_sdk::{stream::StreamManager, SynapClient, SynapConfig};

/// Publisher errors.
#[derive(Debug, thiserror::Error)]
pub enum PublisherError {
    /// Underlying Synap error.
    #[error("synap: {0}")]
    Synap(String),
    /// Internal state error.
    #[error("internal: {0}")]
    Internal(String),
}

/// Abstraction over the durable message bus.
#[async_trait]
pub trait Publisher: Send + Sync + 'static {
    /// Publish one enveloped event onto `stream`. Must be idempotent on
    /// `event_id` in the storage layer — at-least-once delivery is the
    /// contract here.
    async fn publish(&self, stream: &str, envelope: &Value) -> Result<(), PublisherError>;
}

/// Publisher that routes to a live Synap service via `synap-sdk`.
pub struct SynapPublisher {
    streams: StreamManager,
}

impl SynapPublisher {
    /// Connect to `base_url` and return a publisher.
    pub fn new(base_url: &str) -> Result<Self, PublisherError> {
        let cfg = SynapConfig::new(base_url);
        let client = SynapClient::new(cfg).map_err(|e| PublisherError::Synap(e.to_string()))?;
        Ok(Self {
            streams: client.stream(),
        })
    }
}

#[async_trait]
impl Publisher for SynapPublisher {
    async fn publish(&self, stream: &str, envelope: &Value) -> Result<(), PublisherError> {
        let kind = envelope
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("unknown");
        self.streams
            .publish(stream, kind, envelope.clone())
            .await
            .map_err(|e| PublisherError::Synap(e.to_string()))?;
        Ok(())
    }
}

/// In-memory publisher for tests. Records every publish call in order.
#[derive(Default)]
pub struct MemoryPublisher {
    calls: Mutex<Vec<(String, Value)>>,
}

impl MemoryPublisher {
    /// Snapshot the recorded calls.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Count of recorded calls.
    pub fn len(&self) -> usize {
        self.calls.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Whether any calls have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl Publisher for MemoryPublisher {
    async fn publish(&self, stream: &str, envelope: &Value) -> Result<(), PublisherError> {
        self.calls
            .lock()
            .map_err(|_| PublisherError::Internal("memory publisher mutex poisoned".into()))?
            .push((stream.to_string(), envelope.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_publisher_records_calls() {
        let p = MemoryPublisher::default();
        p.publish("cortex.events.raw", &serde_json::json!({ "kind": "turn" }))
            .await
            .unwrap();
        p.publish("cortex.events.raw", &serde_json::json!({ "kind": "tool_call" }))
            .await
            .unwrap();
        let calls = p.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "cortex.events.raw");
    }

}
