//! Synap publisher — writes envelope-compliant events on
//! `cortex.events.bootstrap` (or whatever `--stream` overrides to).
//! Mirrors the `LiveSynapPublisher` shape used by `cortex-fulltext`
//! and `cortex-graph` so operations look identical across all three
//! workers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use synap_sdk::stream::StreamManager;
use synap_sdk::{SynapClient, SynapConfig};

use crate::emitter::BootstrapEvent;

/// Publisher abstraction so tests can use a recording fake.
#[async_trait]
pub trait Publisher: Send + Sync {
    /// Publish one event onto `room`. Returns the Synap-assigned offset
    /// when the SDK exposes it; otherwise an opaque success.
    async fn publish_one(&self, room: &str, event: &BootstrapEvent) -> Result<()>;

    /// Publish a batch of events one-after-another. Default impl walks
    /// `events` in arrival order so the publisher is at-least-once
    /// without needing a multi-item `publish` SDK surface.
    async fn publish_batch(&self, room: &str, events: &[BootstrapEvent]) -> Result<usize> {
        let mut sent = 0usize;
        for event in events {
            self.publish_one(room, event).await?;
            sent += 1;
        }
        Ok(sent)
    }
}

/// Synap client handle built from the bootstrap config / CLI flags.
pub struct SynapHandle {
    streams: StreamManager,
}

impl SynapHandle {
    /// Connect to Synap at `base_url`.
    pub fn new(base_url: &str) -> Result<Self> {
        let cfg = SynapConfig::new(base_url);
        let client =
            SynapClient::new(cfg).map_err(|e| anyhow::anyhow!("synap client: {e}"))?;
        Ok(Self {
            streams: client.stream(),
        })
    }
    /// Shared stream manager.
    pub fn streams(&self) -> &StreamManager {
        &self.streams
    }
}

/// Live Synap publisher.
pub struct LiveSynapPublisher {
    handle: Arc<SynapHandle>,
    /// Maximum retry attempts on transient publish errors.
    max_retry: u32,
}

impl LiveSynapPublisher {
    /// Build a new publisher bound to `handle`.
    pub fn new(handle: Arc<SynapHandle>) -> Self {
        Self {
            handle,
            max_retry: 3,
        }
    }

    /// Override the retry attempt count (useful for tests).
    pub fn with_max_retry(mut self, n: u32) -> Self {
        self.max_retry = n;
        self
    }
}

#[async_trait]
impl Publisher for LiveSynapPublisher {
    async fn publish_one(&self, room: &str, event: &BootstrapEvent) -> Result<()> {
        let payload: Value = serde_json::to_value(event)?;
        let attempts = self.max_retry.max(1);
        let mut last: Option<anyhow::Error> = None;
        let mut attempted_create = false;
        for attempt in 0..attempts {
            match self
                .handle
                .streams()
                .publish(room, &event.kind, payload.clone())
                .await
            {
                Ok(_offset) => return Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    last = Some(anyhow::anyhow!("synap publish: {e}"));
                    if !attempted_create
                        && (msg.contains("not found") || msg.contains("Room"))
                    {
                        attempted_create = true;
                        if let Err(create_err) =
                            self.handle.streams().create_room(room, None).await
                        {
                            tracing::debug!(
                                room,
                                error = %create_err,
                                "stream.create failed; will retry publish anyway"
                            );
                        }
                        continue;
                    }
                    if attempt + 1 == attempts {
                        break;
                    }
                    let backoff_ms = 100u64 << (2 * attempt as u64);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("synap publish exhausted retries")))
    }
}

/// In-memory recording publisher for tests.
#[derive(Default)]
pub struct MemoryPublisher {
    /// Recorded `(room, event)` pairs in arrival order.
    pub calls: Mutex<Vec<(String, BootstrapEvent)>>,
}

impl MemoryPublisher {
    /// Fresh recorder.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot of the recorded calls.
    pub fn snapshot(&self) -> Vec<(String, BootstrapEvent)> {
        self.calls.lock().map(|g| g.clone()).unwrap_or_default()
    }
    /// Total events recorded.
    pub fn count(&self) -> usize {
        self.calls.lock().map(|g| g.len()).unwrap_or(0)
    }
    /// Filter recorded events by kind.
    pub fn by_kind(&self, kind: &str) -> Vec<BootstrapEvent> {
        self.snapshot()
            .into_iter()
            .filter(|(_, e)| e.kind == kind)
            .map(|(_, e)| e)
            .collect()
    }
}

#[async_trait]
impl Publisher for MemoryPublisher {
    async fn publish_one(&self, room: &str, event: &BootstrapEvent) -> Result<()> {
        if let Ok(mut g) = self.calls.lock() {
            g.push((room.to_string(), event.clone()));
        }
        Ok(())
    }
}
