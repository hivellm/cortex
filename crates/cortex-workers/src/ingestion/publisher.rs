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
        // Publish-or-create-then-republish, matching the pattern
        // already in cortex-bootstrap and cortex-classifier-worker.
        // Synap does not auto-create rooms on first publish — sending
        // to a missing room returns "Room … not found", which is
        // *correct* server behaviour. Cortex owns the room lifecycle.
        // Without this fallback every ingestion startup against a
        // fresh Synap drops every envelope until an operator creates
        // the rooms by hand (which is what the 2026-04-28 silent-loss
        // incident traced back to).
        match self.streams.publish(stream, kind, envelope.clone()).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                let looks_like_missing_room = msg.contains("not found") && msg.contains("Room");
                if !looks_like_missing_room {
                    return Err(PublisherError::Synap(msg));
                }
                tracing::warn!(
                    stream,
                    "synap room missing on first publish; creating and retrying"
                );
                if let Err(create_err) = self.streams.create_room(stream, None).await {
                    // The room may already exist (race with another
                    // publisher); only treat the error as fatal if
                    // the second publish ALSO fails.
                    tracing::debug!(
                        stream,
                        error = %create_err,
                        "synap create_room returned an error; will still retry publish"
                    );
                }
                self.streams
                    .publish(stream, kind, envelope.clone())
                    .await
                    .map_err(|e2| {
                        PublisherError::Synap(format!(
                            "publish after create_room still failed: {e2} (initial: {msg})"
                        ))
                    })?;
                Ok(())
            }
        }
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
