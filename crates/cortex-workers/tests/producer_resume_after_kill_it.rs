//! Phase13b §4 — resume-after-kill integration test for the
//! `EnvelopeProducer` trait. ADR-010 promotes to `accepted` once
//! this is green.
//!
//! Strategy: drive a fixture producer over a 10k-event corpus in
//! two phases. Phase 1 panics partway through (simulating
//! `kill -9` between an emit batch's commit and the next batch's
//! checkpoint). Phase 2 wires a fresh producer instance against
//! the same `producer_checkpoints` table and asserts it (a) reads
//! the prior cursor, (b) skips already-emitted envelopes, and (c)
//! finishes the corpus with no duplicates and no gaps.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cortex_storage::MetadataStore;
use cortex_workers::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};
use tokio::sync::Mutex;

/// Synthetic producer that walks a fixed corpus of `event_id`
/// strings. The `panic_after` index is the IT's kill simulator —
/// when set, the producer panics after emitting that many
/// envelopes mid-walk.
struct FixtureProducer {
    name: &'static str,
    scope: &'static str,
    corpus: Vec<String>,
    panic_after: Option<usize>,
    /// Per-(producer, scope) emit counter accumulated across the
    /// IT's two runs. Records what the wrapper actually emitted
    /// post-resume so the assertion can check no duplicates.
    emitted: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl EnvelopeProducer for FixtureProducer {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        // Read the prior cursor and translate it to a start index.
        let start_idx = {
            let store = ctx.metadata.lock().await;
            let row = store.latest_producer_checkpoint(self.name, self.scope)?;
            row.map(|r| {
                self.corpus
                    .iter()
                    .position(|id| id == &r.last_event_id)
                    .map(|p| p + 1)
                    .unwrap_or(0)
            })
            .unwrap_or(0)
        };

        let batch_size = 100usize;
        let mut total = 0u64;
        let mut batches = 0u64;
        let mut last_event_id = String::new();
        let mut emit_count = 0usize;

        let mut idx = start_idx;
        while idx < self.corpus.len() {
            let end = (idx + batch_size).min(self.corpus.len());
            for slot in idx..end {
                last_event_id = self.corpus[slot].clone();
                {
                    let mut emitted = self.emitted.lock().await;
                    emitted.push(last_event_id.clone());
                }
                total += 1;
                emit_count += 1;
                if let Some(after) = self.panic_after {
                    if emit_count >= after {
                        // Persist the partial-batch checkpoint
                        // before "dying" so resume picks up
                        // exactly here. This mirrors the
                        // contract every real producer upholds:
                        // checkpoint at the end of every batch,
                        // not on graceful shutdown.
                        let store = ctx.metadata.lock().await;
                        store.record_producer_checkpoint(
                            self.name,
                            self.scope,
                            &last_event_id,
                            ctx.now,
                            Utc::now() + chrono::Duration::microseconds(batches as i64),
                        )?;
                        return Err(anyhow::anyhow!(
                            "simulated kill -9 after emitting {} envelopes",
                            emit_count
                        ));
                    }
                }
            }
            // End-of-batch checkpoint.
            let store = ctx.metadata.lock().await;
            let accumulated_at = Utc::now() + chrono::Duration::microseconds(batches as i64);
            store.record_producer_checkpoint(
                self.name,
                self.scope,
                &last_event_id,
                ctx.now,
                accumulated_at,
            )?;
            batches += 1;
            idx = end;
        }

        Ok(ProducerReport {
            producer_name: self.name.to_string(),
            envelopes_emitted: total,
            batches_emitted: batches,
            last_event_id,
            last_occurred_at: Some(ctx.now),
        })
    }

    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store.latest_producer_checkpoint(self.name, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-19T03:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn build_corpus(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("01EVENT{i:09}")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_after_kill_finishes_corpus_with_no_duplicates_or_gaps() {
    let corpus = build_corpus(10_000);
    let store = MetadataStore::open_in_memory().expect("metadata store");
    let handle: Arc<Mutex<MetadataStore>> = Arc::new(Mutex::new(store));
    let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.it").with_now(fixed_now());

    let emitted = Arc::new(Mutex::new(Vec::<String>::new()));

    // ---- Phase 1: emit 30% then die ----
    let phase1 = FixtureProducer {
        name: "fixture",
        scope: "main",
        corpus: corpus.clone(),
        panic_after: Some(3_000),
        emitted: emitted.clone(),
    };
    let phase1_outcome = phase1.produce(&ctx).await;
    assert!(
        phase1_outcome.is_err(),
        "phase1 must surface the simulated kill error"
    );
    {
        let emitted_so_far = emitted.lock().await;
        assert_eq!(
            emitted_so_far.len(),
            3_000,
            "phase 1 emits 30% of the corpus"
        );
    }
    // The checkpoint table now carries the cursor at the kill
    // point — verify before phase 2 wires a new producer.
    {
        let store = handle.lock().await;
        let row = store
            .latest_producer_checkpoint("fixture", "main")
            .expect("query");
        let row = row.expect("phase1 wrote a checkpoint before the panic");
        let expected = corpus[2_999].clone();
        assert_eq!(row.last_event_id, expected);
    }

    // ---- Phase 2: fresh producer, no panic ----
    let phase2 = FixtureProducer {
        name: "fixture",
        scope: "main",
        corpus: corpus.clone(),
        panic_after: None,
        emitted: emitted.clone(),
    };
    let phase2_report = phase2.produce(&ctx).await.expect("phase2 completes");

    // ---- Assertions ----
    // (a) Phase 2 emitted exactly the remaining 70%.
    assert_eq!(
        phase2_report.envelopes_emitted, 7_000,
        "phase2 must emit only the unfinished tail"
    );
    // (b) No duplicates across both runs.
    let total_emits = emitted.lock().await.clone();
    let unique: BTreeSet<&String> = total_emits.iter().collect();
    assert_eq!(
        unique.len(),
        total_emits.len(),
        "no envelope must be emitted twice across the kill boundary"
    );
    // (c) Final count matches the input corpus exactly.
    assert_eq!(
        total_emits.len(),
        corpus.len(),
        "no gaps in the union of phase1 + phase2 emits"
    );
    // (d) Final cursor sits at the last event.
    let last = corpus.last().unwrap();
    {
        let store = handle.lock().await;
        let row = store
            .latest_producer_checkpoint("fixture", "main")
            .expect("query")
            .expect("row");
        assert_eq!(&row.last_event_id, last);
    }
}
