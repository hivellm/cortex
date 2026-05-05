//! Phase11p §4.2 — live-topic IT.
//!
//! Seeds 12 turn envelopes carrying inline `payload.embedding`
//! arrays across two synthetic clusters (cluster A: 7 turns,
//! cluster B: 5 turns, plus one outlier) into a temp parquet
//! archive. Runs `LiveTopicSource::fetch` and asserts:
//!
//! - 2 clusters returned,
//! - the outlier is dropped as noise (label = -1),
//! - cluster sizes 7 and 5 (one ClusterSession per session_id),
//! - deterministic ordering when run twice.
//!
//! Gated on `CORTEX_CONSOLIDATOR_LIVE_IT=1`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use cortex_core::events::{Context, Envelope, Kind, Stream};
use cortex_workers::consolidator::source::LiveTopicSource;

fn it_enabled() -> bool {
    std::env::var("CORTEX_CONSOLIDATOR_LIVE_IT").as_deref() == Ok("1")
}

fn turn_with_embedding(
    event_id: &str,
    session_id: &str,
    occurred_at: &str,
    embedding: Vec<f64>,
) -> Envelope {
    let mut payload = serde_json::json!({
        "user_message": format!("msg {event_id}"),
        "tool_call_event_ids": [],
    });
    payload.as_object_mut().unwrap().insert(
        "embedding".to_string(),
        serde_json::Value::Array(
            embedding
                .into_iter()
                .map(|x| {
                    serde_json::Number::from_f64(x)
                        .map(serde_json::Value::Number)
                        .unwrap()
                })
                .collect(),
        ),
    );
    Envelope {
        event_id: event_id.into(),
        schema_version: "1".into(),
        occurred_at: occurred_at.into(),
        ingested_at: None,
        session_id: session_id.into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: None,
        kind: Kind::Turn,
        context: Context {
            repo: Some("cortex".into()),
            branch: None,
            commit: None,
            cwd: None,
            user: None,
            platform: "linux".into(),
            ide: None,
            extras: BTreeMap::new(),
        },
        payload,
        redactions: vec![],
        content_hash:
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .into(),
        parent_event_id: None,
    }
}

fn write_archive(root: &Path, envelopes: &[Envelope]) {
    let dir = root.join("events/year=2026/month=04/day=26/hour=19");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("raw-00000.parquet");
    let file = File::create(&path).unwrap();
    let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
    for env in envelopes {
        let line = serde_json::to_string(env).unwrap();
        enc.write_all(line.as_bytes()).unwrap();
        enc.write_all(b"\n").unwrap();
    }
    enc.finish().unwrap();
}

const SINCE_MS: i64 = 1_777_230_000_000; // 2026-04-26T19:00:00Z
const UNTIL_MS: i64 = 1_777_233_600_000; // 2026-04-26T20:00:00Z

#[test]
fn live_topic_source_splits_two_clusters_and_drops_outlier() {
    if !it_enabled() {
        eprintln!("skipping consolidator_live_topic_it (CORTEX_CONSOLIDATOR_LIVE_IT != 1)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let mut envelopes: Vec<Envelope> = Vec::new();

    // Cluster A: 7 turns near (1, 1).
    for i in 0..7 {
        envelopes.push(turn_with_embedding(
            &format!("A{i}"),
            &format!("SA{i}"),
            &format!("2026-04-26T19:{:02}:00Z", i),
            vec![1.0 + (i as f64) * 0.01, 1.0 + (i as f64) * 0.02],
        ));
    }
    // Cluster B: 5 turns near (10, 10).
    for i in 0..5 {
        envelopes.push(turn_with_embedding(
            &format!("B{i}"),
            &format!("SB{i}"),
            &format!("2026-04-26T19:{:02}:00Z", 10 + i),
            vec![10.0 + (i as f64) * 0.01, 10.0 + (i as f64) * 0.02],
        ));
    }
    // 1 outlier far from both.
    envelopes.push(turn_with_embedding(
        "OUT",
        "SOUT",
        "2026-04-26T19:30:00Z",
        vec![50.0, 50.0],
    ));
    write_archive(dir.path(), &envelopes);

    let source = LiveTopicSource::new(dir.path(), 3);
    let r1 = source.fetch("cortex", SINCE_MS, UNTIL_MS).expect("fetch");
    assert_eq!(r1.len(), 2, "expected exactly 2 clusters");

    let total_sessions: usize = r1.iter().map(|c| c.sessions.len()).sum();
    assert_eq!(total_sessions, 12, "outlier must be dropped (7 + 5 = 12)");

    let sizes: Vec<usize> = {
        let mut s: Vec<usize> = r1.iter().map(|c| c.sessions.len()).collect();
        s.sort();
        s
    };
    assert_eq!(sizes, vec![5, 7]);

    // Deterministic ordering across runs.
    let r2 = source.fetch("cortex", SINCE_MS, UNTIL_MS).expect("fetch");
    let labels1: Vec<&str> = r1.iter().map(|c| c.label.as_str()).collect();
    let labels2: Vec<&str> = r2.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels1, labels2);
}
