//! [`OpenCodeProducer`] — `impl EnvelopeProducer` for the OpenCode
//! HTTP hook transport.
//!
//! The producer owns the MPSC receiver that the HTTP server forwards
//! inbound envelopes into. Each `produce` call drains the channel
//! non-blocking, publishes every envelope via the injected `Publisher`,
//! then writes one `producer_checkpoints` row per distinct session_id
//! seen in the batch.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_adapter_claude_code::publisher::Publisher;
use cortex_core::events::Envelope;
use cortex_workers::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};
use tokio::sync::{mpsc, Mutex};

/// Stable producer name. Must match the `producer_name` key written
/// to `producer_checkpoints` — changing it loses resume continuity.
pub const OPENCODE_PRODUCER_NAME: &str = "opencode";

/// MPSC-based `EnvelopeProducer` for the OpenCode plugin transport.
///
/// Constructed by the future daemon via
/// `OpenCodeProducer::new(publisher, receiver)`.  The HTTP server
/// forwards `(session_id, Envelope)` pairs into the sender half;
/// `produce` drains the receiver end in a non-blocking sweep.
pub struct OpenCodeProducer {
    /// MPSC receiver. Wrapped in `Mutex` so the struct can be `Sync`
    /// while holding the non-`Sync` receiver.
    rx: Mutex<mpsc::Receiver<(String, Envelope)>>,
    /// Downstream publisher — forwards envelopes to
    /// `cortex-ingestion /v1/events/batch`.
    publisher: Arc<dyn Publisher>,
}

impl OpenCodeProducer {
    /// Create a new producer wired to `publisher` and `rx`.
    pub fn new(publisher: Arc<dyn Publisher>, rx: mpsc::Receiver<(String, Envelope)>) -> Self {
        Self {
            rx: Mutex::new(rx),
            publisher,
        }
    }
}

#[async_trait]
impl EnvelopeProducer for OpenCodeProducer {
    fn name(&self) -> &'static str {
        OPENCODE_PRODUCER_NAME
    }

    /// Drain the MPSC channel, publish every envelope, write one
    /// checkpoint per distinct `session_id`, and return the run
    /// summary.
    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        let mut rx = self.rx.lock().await;

        // Per-session tracking: last_event_id for each session seen.
        let mut session_cursors: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut envelopes_emitted: u64 = 0;
        let mut last_event_id = String::new();

        // Non-blocking drain — take everything currently queued.
        loop {
            match rx.try_recv() {
                Ok((session_id, envelope)) => {
                    last_event_id = envelope.event_id.clone();
                    session_cursors.insert(session_id, last_event_id.clone());
                    self.publisher.publish(envelope).await;
                    envelopes_emitted += 1;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        // Flush whatever the publisher queued during this batch.
        self.publisher.flush().await;

        // Write one checkpoint row per session_id seen.
        if !session_cursors.is_empty() {
            let store = ctx.metadata.lock().await;
            for (session_id, event_id) in &session_cursors {
                store
                    .record_producer_checkpoint(
                        OPENCODE_PRODUCER_NAME,
                        session_id,
                        event_id,
                        ctx.now,
                        ctx.now,
                    )
                    .map_err(|e| anyhow::anyhow!("record_producer_checkpoint failed: {e}"))?;
            }
        }

        Ok(ProducerReport {
            producer_name: OPENCODE_PRODUCER_NAME.to_string(),
            envelopes_emitted,
            batches_emitted: 1,
            last_event_id,
            last_occurred_at: None,
        })
    }

    /// Return the latest checkpoint for `(opencode, scope)` where
    /// `scope` is the `session_id`.
    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store
            .latest_producer_checkpoint(OPENCODE_PRODUCER_NAME, scope)
            .map_err(|e| anyhow::anyhow!("latest_producer_checkpoint failed: {e}"))?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_adapter_claude_code::publisher::MemoryPublisher;

    fn make_producer() -> (OpenCodeProducer, mpsc::Sender<(String, Envelope)>) {
        let (tx, rx) = mpsc::channel(64);
        let publisher = Arc::new(MemoryPublisher::new());
        let producer = OpenCodeProducer::new(publisher, rx);
        (producer, tx)
    }

    #[test]
    fn name_returns_opencode() {
        let (producer, _tx) = make_producer();
        assert_eq!(producer.name(), "opencode");
    }
}
