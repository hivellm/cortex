//! Phase11o §4.2 — pruner safety integration test.
//!
//! Asserts that no `source_event_id` referenced by an active
//! (non-expired) consolidation is dropped from any backend before
//! the consolidation itself expires. Catches a class of regressions
//! where an aggressive cold-tier strip would orphan the
//! consolidation summaries that the dashboard's "Consolidated
//! context" lane reads.
//!
//! Gated on `CORTEX_PRUNER_IT=1`. Default `cargo test` skips it.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use cortex_storage::names::{
    COLLECTION_COLD_BINARY, COLLECTION_CONSOLIDATION_FP32, COLLECTION_CONSOLIDATION_PQ,
    INDEX_CONSOLIDATIONS,
};
use cortex_workers::embedder::vectorizer_client::MemoryVectorizerClient;
use cortex_workers::pruner::engine::{run_sweep, ConsolidationDoc};
use cortex_workers::pruner::meili_sink::{MeiliPruneError, MeiliPruneOps};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct StubMeili {
    docs: Mutex<HashMap<String, serde_json::Map<String, Value>>>,
}

#[async_trait]
impl MeiliPruneOps for StubMeili {
    async fn update_documents(&self, _index: &str, docs: &[Value]) -> Result<(), MeiliPruneError> {
        let mut g = self.docs.lock().unwrap();
        for d in docs {
            let map = match d.as_object() {
                Some(m) => m,
                None => continue,
            };
            let id = match map.get("event_id").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let entry = g.entry(id).or_default();
            for (k, v) in map.iter() {
                entry.insert(k.clone(), v.clone());
            }
        }
        Ok(())
    }
    async fn delete_documents(&self, _index: &str, ids: &[String]) -> Result<(), MeiliPruneError> {
        let mut g = self.docs.lock().unwrap();
        for id in ids {
            g.remove(id);
        }
        Ok(())
    }
}

fn it_enabled() -> bool {
    std::env::var("CORTEX_PRUNER_IT").as_deref() == Ok("1")
}

#[tokio::test]
async fn active_consolidation_vectors_survive_demotion() {
    if !it_enabled() {
        eprintln!("skipping pruner_safety_it (CORTEX_PRUNER_IT != 1)");
        return;
    }

    let now = Utc::now();
    let vec_client = MemoryVectorizerClient::default();
    let meili = StubMeili::default();

    // Mix of consolidations:
    //  - active-hot     :   3 d old  → no action (still hot)
    //  - active-warm    :  30 d old  → moves hot→warm; vectors must
    //                                  exist in `cortex.consolidation.pq`
    //                                  after the sweep (NOT lost).
    //  - active-cold    : 200 d old  → moves warm→cold; vectors must
    //                                  land in `cortex.cold.binary`.
    //  - expired        : 500 d old  → meili row purged; we still
    //                                  expect the COLD vectors (if any)
    //                                  to survive — only the meili
    //                                  side is hard-purged here, the
    //                                  vector hard-purge runs through
    //                                  the `/cortex forget` MCP path.
    let cases: &[(&str, i64, &str)] = &[
        ("active-hot", 3, COLLECTION_CONSOLIDATION_FP32),
        ("active-warm", 30, COLLECTION_CONSOLIDATION_FP32),
        ("active-cold", 200, COLLECTION_CONSOLIDATION_PQ),
        ("expired", 500, COLLECTION_COLD_BINARY),
    ];
    let mut docs: Vec<ConsolidationDoc> = Vec::new();
    for (label, age, src) in cases {
        let event_id = format!("cons-{label}");
        let vec_id = format!("v-{label}");
        {
            let mut stored = vec_client.dedup_keys_per_collection.lock().unwrap();
            stored
                .entry((*src).into())
                .or_default()
                .insert(
                    vec_id.clone(),
                    cortex_workers::embedder::vectorizer_client::StoredVec {
                        dedup_key: vec_id.clone(),
                        server_id: format!("srv-{vec_id}"),
                    },
                );
        }
        // Seed the meili row with the consolidation metadata.
        {
            let mut g = meili.docs.lock().unwrap();
            let mut map = serde_json::Map::new();
            map.insert("event_id".into(), Value::String(event_id.clone()));
            map.insert(
                "occurred_at".into(),
                Value::String((now - Duration::days(*age)).to_rfc3339()),
            );
            map.insert("body".into(), Value::String("active body".into()));
            g.insert(event_id.clone(), map);
        }
        docs.push(ConsolidationDoc {
            event_id,
            occurred_at: now - Duration::days(*age),
            vector_ids: vec![vec_id],
        });
    }

    let _report = run_sweep(&docs, now, &vec_client, &meili, INDEX_CONSOLIDATIONS)
        .await
        .expect("sweep");

    // Invariant 1: active-hot vectors stay in fp32 untouched.
    assert!(
        vec_client
            .dedup_keys_per_collection
            .lock()
            .unwrap()
            .get(COLLECTION_CONSOLIDATION_FP32)
            .map(|m| m.contains_key("v-active-hot"))
            .unwrap_or(false),
        "active-hot vector must NOT have moved",
    );

    // Invariant 2: active-warm vectors landed in PQ (NOT lost).
    assert!(
        vec_client
            .dedup_keys_per_collection
            .lock()
            .unwrap()
            .get(COLLECTION_CONSOLIDATION_PQ)
            .map(|m| m.contains_key("v-active-warm"))
            .unwrap_or(false),
        "active-warm vector must exist in PQ after demotion",
    );

    // Invariant 3: active-cold vectors landed in cold.binary
    // (NOT lost). The active cold consolidation referenced a
    // vector originally in the PQ collection.
    assert!(
        vec_client
            .dedup_keys_per_collection
            .lock()
            .unwrap()
            .get(COLLECTION_COLD_BINARY)
            .map(|m| m.contains_key("v-active-cold"))
            .unwrap_or(false),
        "active-cold vector must exist in cold.binary after demotion",
    );

    // Invariant 4: meili rows for active consolidations stay
    // present (only `expired` is hard-purged).
    let g = meili.docs.lock().unwrap();
    for active in ["cons-active-hot", "cons-active-warm", "cons-active-cold"] {
        assert!(
            g.contains_key(active),
            "active consolidation {active} must NOT have been deleted",
        );
    }
    assert!(
        !g.contains_key("cons-expired"),
        "expired consolidation must have been hard-purged",
    );
}
