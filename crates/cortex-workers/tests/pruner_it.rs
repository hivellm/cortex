//! Phase11o §4.1 — pruner integration test.
//!
//! Seeds 100 source events spanning the five age buckets used by
//! the consolidation tier (0–7 d / 7–90 d / 90–365 d / >365 d / a
//! "fresh" 0 d slice) plus 20 `Kind::Consolidation` rows that
//! reference them. Asserts post-prune doc counts in every backend
//! match the expected per-tier targets.
//!
//! Gated on `CORTEX_PRUNER_IT=1`. Running without the gate is a
//! silent skip so the default `cargo test` stays fast.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
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

/// Minimal Meili stand-in that records every update + delete so
/// the assertions can verify per-row outcomes. Implements the
/// pruner's narrow `MeiliPruneOps` trait so the engine path is
/// exercised end-to-end.
#[derive(Default)]
struct StubMeili {
    /// `event_id` → merged document. Mirrors Meili's "merge by
    /// primary key" semantics: each `update_documents` call
    /// rewrites only the keys present in the input.
    docs: Mutex<HashMap<String, serde_json::Map<String, Value>>>,
}

impl StubMeili {
    fn seed(&self, event_id: &str, occurred_at: DateTime<Utc>, vector_ids: Vec<String>) {
        let mut g = self.docs.lock().unwrap();
        let mut map = serde_json::Map::new();
        map.insert("event_id".into(), Value::String(event_id.into()));
        map.insert(
            "occurred_at".into(),
            Value::String(occurred_at.to_rfc3339()),
        );
        map.insert(
            "vector_ids".into(),
            Value::Array(vector_ids.into_iter().map(Value::String).collect()),
        );
        // High-cost fields the pruner's cold-tier path strips.
        map.insert("body".into(), Value::String("body…".into()));
        map.insert("summary".into(), Value::String("summary…".into()));
        map.insert(
            "outcome_distribution".into(),
            serde_json::json!({"success": 0.7, "failure": 0.3}),
        );
        g.insert(event_id.into(), map);
    }
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
            let id = map
                .get("event_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let id = match id {
                Some(s) => s,
                None => continue,
            };
            let entry = g.entry(id).or_insert_with(serde_json::Map::new);
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
async fn prune_seeds_100_events_and_demotes_correctly() {
    if !it_enabled() {
        eprintln!("skipping pruner_it (CORTEX_PRUNER_IT != 1)");
        return;
    }

    let now = Utc::now();
    let vec_client = MemoryVectorizerClient::default();
    let meili = StubMeili::default();

    // Seed 100 source events distributed across 5 age buckets:
    //  - 20 fresh   (0 d)        → hot
    //  - 20 recent  (3 d)        → hot
    //  - 20 warm    (30 d)       → warm
    //  - 20 cold    (200 d)      → cold
    //  - 20 expired (500 d)      → expired
    //
    // The consolidator writes each event's vector under its
    // canonical primary key (`event_id`) into the matching tier
    // collection. We replay that here so the move/delete path has
    // real data to operate on.
    let buckets: &[(i64, &str)] = &[
        (0, COLLECTION_CONSOLIDATION_FP32),
        (3, COLLECTION_CONSOLIDATION_FP32),
        (30, COLLECTION_CONSOLIDATION_FP32),
        (200, COLLECTION_CONSOLIDATION_PQ),
        (500, COLLECTION_COLD_BINARY),
    ];
    let mut docs: Vec<ConsolidationDoc> = Vec::new();
    let mut consolidation_idx = 0u32;
    for (age_days, src_collection) in buckets {
        for slot in 0..20 {
            let event_id = format!("evt-{age_days:03}-{slot:02}");
            // Place the event's vector in the matching collection.
            {
                let mut stored = vec_client.dedup_keys_per_collection.lock().unwrap();
                stored
                    .entry((*src_collection).into())
                    .or_default()
                    .insert(event_id.clone(), format!("srv-{event_id}"));
            }

            // Group every 5 source events into one consolidation
            // (so 100 events → 20 consolidations). The
            // consolidation's `occurred_at` matches the source
            // event's age so the engine buckets it identically.
            if slot % 5 == 0 {
                let consolidation_id = format!("cons-{consolidation_idx:03}");
                consolidation_idx += 1;
                let occurred_at = now - Duration::days(*age_days);
                let vector_ids: Vec<String> = (0..5)
                    .map(|s| format!("evt-{age_days:03}-{:02}", slot + s))
                    .collect();
                meili.seed(&consolidation_id, occurred_at, vector_ids.clone());
                docs.push(ConsolidationDoc {
                    event_id: consolidation_id,
                    occurred_at,
                    vector_ids,
                });
            }
        }
    }
    assert_eq!(docs.len(), 20, "20 consolidations seeded");

    let report = run_sweep(&docs, now, &vec_client, &meili, INDEX_CONSOLIDATIONS)
        .await
        .expect("sweep");

    assert_eq!(report.consolidations_seen, 20);

    // Tier-pair counts. 4 consolidations per bucket, 5 vectors
    // each ⇒ 20 vectors per move pair (when source data is
    // present in the source collection):
    //   - 4 consolidations × 5 vectors at age 30  → hot→warm = 20
    //   - 4 consolidations × 5 vectors at age 200 → warm→cold = 20
    //   - 4 consolidations at age 500 → expired (purge, no move)
    assert_eq!(
        report
            .events_demoted_per_tier
            .get("hot->warm")
            .copied()
            .unwrap_or(0),
        20,
        "hot→warm: 4 consolidations × 5 vectors",
    );
    assert_eq!(
        report
            .events_demoted_per_tier
            .get("warm->cold")
            .copied()
            .unwrap_or(0),
        20,
        "warm→cold: 4 consolidations × 5 vectors",
    );
    assert_eq!(report.events_purged, 4, "4 expired consolidations purged");

    // Vectorizer-side residue: the warm-tier transition pulled 20
    // ids from `cortex.consolidation.fp32` (60 fresh + 0 d hot
    // entries stay; 60 - 0 = 60).  The .pq collection ends up
    // with 20 newly-arrived (warm→cold path consumed all original
    // PQ entries which were 200 d old).  The cold collection
    // takes the 20 warm→cold arrivals + the original 20 cold
    // entries (which warm→cold doesn't touch — those are 500 d
    // and route to expired purge).
    let fp32_left = vec_client.stored_count_for(COLLECTION_CONSOLIDATION_FP32);
    let pq_left = vec_client.stored_count_for(COLLECTION_CONSOLIDATION_PQ);
    let cold_left = vec_client.stored_count_for(COLLECTION_COLD_BINARY);
    assert_eq!(fp32_left, 40, "20 fresh + 20 recent stay in fp32");
    assert_eq!(pq_left, 20, "20 hot→warm arrivals (original PQ all moved)");
    assert_eq!(cold_left, 40, "20 warm→cold arrivals + 20 originals");

    // Meili-side: every cold-tier consolidation lost its
    // `body`/`summary`/`outcome_distribution` fields. Expired
    // consolidations are deleted entirely.
    let g = meili.docs.lock().unwrap();
    assert_eq!(g.len(), 16, "20 seeded - 4 expired = 16");
    for (id, map) in g.iter() {
        if id.starts_with("cons-")
            && (12..=15).contains(&id.trim_start_matches("cons-").parse::<u32>().unwrap_or(99))
        {
            // The 4 cold-tier consolidations (indexes 12..15 — the
            // 4th bucket in seed order) were stripped.
            assert_eq!(map.get("body"), Some(&Value::Null), "{id} body cleared");
            assert_eq!(
                map.get("summary"),
                Some(&Value::Null),
                "{id} summary cleared"
            );
            assert_eq!(
                map.get("outcome_distribution"),
                Some(&Value::Null),
                "{id} outcome_distribution cleared"
            );
        }
    }
}
