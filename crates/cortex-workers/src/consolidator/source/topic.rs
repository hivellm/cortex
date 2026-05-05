//! Phase11p §1.3 — Live topic source.
//!
//! Reads turn envelopes from the parquet archive within a per-repo
//! time window, runs HDBSCAN over each turn's inline
//! `payload.embedding` array, and emits one
//! [`crate::consolidator::producer::topic::TopicCluster`] per
//! non-noise label.
//!
//! Turns without an inline `payload.embedding` are dropped before
//! clustering — the lossy filter is intentional because the
//! Vectorizer-side fetch path (which would let us recover those
//! vectors) is reserved for a follow-up. In practice the embedder
//! stamps every turn's vector under `metadata.dedup_key`; whether
//! to surface that on the wire envelope is an open design call
//! tracked separately from this task.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cortex_core::events::{Envelope, Kind};
use hdbscan::{Hdbscan, HdbscanHyperParams};

use crate::consolidator::producer::topic::{ClusterSession, TopicCluster, MIN_CLUSTER_SIZE};

use super::SourceError;

/// Live source backed by the parquet archive + HDBSCAN.
#[derive(Debug, Clone)]
pub struct LiveTopicSource {
    archive_root: PathBuf,
    /// HDBSCAN `min_cluster_size`. Defaults to
    /// [`MIN_CLUSTER_SIZE`] (3) — the consolidator's lower bound.
    /// Operators tune via `--min-cluster-size` on the bin.
    min_cluster_size: usize,
}

impl LiveTopicSource {
    /// Build a new source. `min_cluster_size = 0` falls back to the
    /// consolidator's [`MIN_CLUSTER_SIZE`] so the bin can pass `0`
    /// when the operator has no override.
    pub fn new(archive_root: impl Into<PathBuf>, min_cluster_size: usize) -> Self {
        let mcs = if min_cluster_size == 0 {
            MIN_CLUSTER_SIZE
        } else {
            min_cluster_size
        };
        Self {
            archive_root: archive_root.into(),
            min_cluster_size: mcs,
        }
    }

    /// Materialise topic clusters in the `[since_ms, until_ms]`
    /// window for the given repo. Empty result is `Ok(vec![])`.
    pub fn fetch(
        &self,
        repo: &str,
        since_ms: i64,
        until_ms: i64,
    ) -> Result<Vec<TopicCluster>, SourceError> {
        let envelopes = cortex_storage::archive::walk_envelopes(&self.archive_root, |env| {
            if env.kind != Kind::Turn {
                return false;
            }
            if env.context.repo.as_deref() != Some(repo) {
                return false;
            }
            let ts = match chrono::DateTime::parse_from_rfc3339(&env.occurred_at) {
                Ok(t) => t.timestamp_millis(),
                Err(_) => return false,
            };
            ts >= since_ms && ts <= until_ms
        })?;
        if envelopes.is_empty() {
            return Ok(Vec::new());
        }

        // Pull the inline embedding off each envelope. Drop turns
        // without one — the lossy filter is intentional (see the
        // module docstring).
        let mut keep: Vec<&Envelope> = Vec::new();
        let mut points: Vec<Vec<f64>> = Vec::new();
        for env in &envelopes {
            if let Some(emb) = read_inline_embedding(env) {
                keep.push(env);
                points.push(emb);
            }
        }
        if points.len() < self.min_cluster_size {
            return Ok(Vec::new());
        }

        let hp = HdbscanHyperParams::builder()
            .min_cluster_size(self.min_cluster_size)
            // Phase11p §1.3 — pin `min_samples = 1` so a tight
            // cluster of `min_cluster_size` points (the spec floor)
            // cannot be rejected by the core-distance gate. With
            // the default `min_samples = min_cluster_size`, every
            // turn would need ≥ `min_cluster_size` similarly-distant
            // neighbours which is too strict for our short windows.
            .min_samples(1)
            .build();
        let clusterer = Hdbscan::new(&points, hp);
        let labels = clusterer
            .cluster()
            .map_err(|e| SourceError::Cluster(e.to_string()))?;

        // Group envelopes by cluster label. Negative labels (HDBSCAN
        // noise marker) are dropped.
        let mut grouped: BTreeMap<i32, Vec<&Envelope>> = BTreeMap::new();
        for (idx, label) in labels.iter().enumerate() {
            if *label < 0 {
                continue;
            }
            grouped.entry(*label).or_default().push(keep[idx]);
        }

        let mut clusters: Vec<TopicCluster> = Vec::new();
        for (label, members) in grouped {
            if members.len() < self.min_cluster_size {
                continue;
            }
            let sessions = build_cluster_sessions(&members);
            if sessions.is_empty() {
                continue;
            }
            clusters.push(TopicCluster {
                label: format!("topic-{label}"),
                repo: repo.to_string(),
                sessions,
            });
        }
        // Deterministic ordering across runs: by label string.
        clusters.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(clusters)
    }
}

/// Read `payload.embedding` as a `Vec<f64>`. Returns `None` when the
/// field is absent or the array contains a non-numeric entry.
fn read_inline_embedding(env: &Envelope) -> Option<Vec<f64>> {
    let arr = env.payload.get("embedding")?.as_array()?;
    let mut out: Vec<f64> = Vec::with_capacity(arr.len());
    for v in arr {
        out.push(v.as_f64()?);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Group cluster members by `session_id` and project each group
/// onto a [`ClusterSession`]. The per-session metadata
/// (`start_ms` / `end_ms` / `outcome_distribution` /
/// `one_line_digest`) is derived from the matched envelopes alone
/// — no extra archive walk needed.
fn build_cluster_sessions(envelopes: &[&Envelope]) -> Vec<ClusterSession> {
    let mut by_session: BTreeMap<String, Vec<&Envelope>> = BTreeMap::new();
    for env in envelopes {
        by_session
            .entry(env.session_id.clone())
            .or_default()
            .push(env);
    }
    let mut out: Vec<ClusterSession> = Vec::new();
    for (session_id, members) in by_session {
        let mut start_ms = i64::MAX;
        let mut end_ms = i64::MIN;
        let mut outcome_distribution: BTreeMap<String, u32> = BTreeMap::new();
        let mut digest: Option<String> = None;
        for env in &members {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&env.occurred_at) {
                let ms = t.timestamp_millis();
                start_ms = start_ms.min(ms);
                end_ms = end_ms.max(ms);
            }
            if let Some(s) = env.payload.get("outcome").and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    *outcome_distribution.entry(s.to_string()).or_insert(0) += 1;
                }
            }
            if digest.is_none() {
                if let Some(s) = env.payload.get("user_message").and_then(|v| v.as_str()) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        digest = Some(clip(trimmed, 200).to_string());
                    }
                }
            }
        }
        if start_ms == i64::MAX || end_ms == i64::MIN {
            // No parseable timestamps — skip the session row.
            continue;
        }
        out.push(ClusterSession {
            session_id,
            start_ms,
            end_ms,
            outcome_distribution,
            one_line_digest: digest.unwrap_or_default(),
        });
    }
    out.sort_by_key(|s| s.start_ms);
    out
}

fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::{Context, Stream};
    use std::collections::BTreeMap as Bm;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    fn turn_with_embedding(
        event_id: &str,
        session_id: &str,
        repo: &str,
        occurred_at: &str,
        embedding: Vec<f64>,
        user_message: &str,
    ) -> Envelope {
        let mut payload = serde_json::json!({
            "user_message": user_message,
            "tool_call_event_ids": [],
        });
        payload.as_object_mut().unwrap().insert(
            "embedding".to_string(),
            serde_json::Value::Array(
                embedding
                    .into_iter()
                    .map(|x| serde_json::Number::from_f64(x).map(serde_json::Value::Number).unwrap())
                    .collect(),
            ),
        );
        Envelope {
            event_id: event_id.to_string(),
            schema_version: "1".to_string(),
            occurred_at: occurred_at.to_string(),
            ingested_at: None,
            session_id: session_id.to_string(),
            stream: Stream::Live,
            tool: "claude-code".to_string(),
            model: None,
            kind: Kind::Turn,
            context: Context {
                repo: Some(repo.to_string()),
                branch: None,
                commit: None,
                cwd: None,
                user: None,
                platform: "linux".to_string(),
                ide: None,
                extras: Bm::new(),
            },
            payload,
            redactions: Vec::new(),
            content_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
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

    /// Window spanning the whole 2026-04-26 hour=19.
    const SINCE_MS: i64 = 1_777_230_000_000; // 2026-04-26T19:00:00Z
    const UNTIL_MS: i64 = 1_777_233_600_000; // 2026-04-26T20:00:00Z

    #[test]
    fn fetch_empty_archive_returns_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("events")).unwrap();
        let src = LiveTopicSource::new(dir.path(), 0);
        let got = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn fetch_sub_threshold_returns_empty() {
        // Only 2 turns < MIN_CLUSTER_SIZE = 3 ⇒ no clusters.
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                turn_with_embedding(
                    "T1",
                    "S1",
                    "cortex",
                    "2026-04-26T19:01:00Z",
                    vec![1.0, 1.0],
                    "hi",
                ),
                turn_with_embedding(
                    "T2",
                    "S2",
                    "cortex",
                    "2026-04-26T19:02:00Z",
                    vec![1.1, 1.0],
                    "bye",
                ),
            ],
        );
        let src = LiveTopicSource::new(dir.path(), 0);
        let got = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn fetch_two_clusters_split() {
        let dir = tempfile::tempdir().unwrap();
        // Cluster A: 5 tightly-grouped turns near (1,1).
        // Cluster B: 5 tightly-grouped turns near (10,10).
        // 1 outlier near (50, 50) → noise.
        let mut envelopes = Vec::new();
        for i in 0..5 {
            envelopes.push(turn_with_embedding(
                &format!("A{i}"),
                &format!("SA{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", i),
                vec![1.0 + (i as f64) * 0.01, 1.0 + (i as f64) * 0.02],
                &format!("a{i}"),
            ));
        }
        for i in 0..5 {
            envelopes.push(turn_with_embedding(
                &format!("B{i}"),
                &format!("SB{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", 10 + i),
                vec![10.0 + (i as f64) * 0.01, 10.0 + (i as f64) * 0.02],
                &format!("b{i}"),
            ));
        }
        envelopes.push(turn_with_embedding(
            "OUT",
            "SOUT",
            "cortex",
            "2026-04-26T19:30:00Z",
            vec![50.0, 50.0],
            "outlier",
        ));
        write_archive(dir.path(), &envelopes);
        let src = LiveTopicSource::new(dir.path(), 3);
        let got = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        assert_eq!(got.len(), 2, "expected two clusters, got {}", got.len());
        let total_sessions: usize = got.iter().map(|c| c.sessions.len()).sum();
        assert_eq!(
            total_sessions, 10,
            "outlier must be dropped as noise (label = -1)",
        );
    }

    #[test]
    fn fetch_label_stable_across_runs() {
        // Same data + same min_cluster_size ⇒ same labels every
        // run (HDBSCAN is deterministic on a fixed dataset; this
        // test pins the contract so a future seed flip never
        // silently changes the cluster ordering). Two clusters of
        // 3 points each so HDBSCAN actually returns non-noise
        // labels — single-blob fixtures collapse to all-noise and
        // would let a regression slip through.
        let dir = tempfile::tempdir().unwrap();
        let mut envelopes = Vec::new();
        for i in 0..3 {
            envelopes.push(turn_with_embedding(
                &format!("A{i}"),
                &format!("SA{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", i),
                vec![1.0 + (i as f64) * 0.01, 1.0 + (i as f64) * 0.02],
                &format!("a{i}"),
            ));
        }
        for i in 0..3 {
            envelopes.push(turn_with_embedding(
                &format!("B{i}"),
                &format!("SB{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", 10 + i),
                vec![10.0 + (i as f64) * 0.01, 10.0 + (i as f64) * 0.02],
                &format!("b{i}"),
            ));
        }
        write_archive(dir.path(), &envelopes);
        let src = LiveTopicSource::new(dir.path(), 3);
        let r1 = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        let r2 = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        assert!(!r1.is_empty(), "fixture must produce ≥ 1 cluster");
        let labels1: Vec<&str> = r1.iter().map(|c| c.label.as_str()).collect();
        let labels2: Vec<&str> = r2.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels1, labels2);
    }

    #[test]
    fn fetch_repo_filter_respected() {
        let dir = tempfile::tempdir().unwrap();
        let mut envelopes = Vec::new();
        // 6 cortex turns split into two tight clusters A + B so
        // HDBSCAN actually returns a non-empty cluster set we can
        // inspect for repo leakage.
        for i in 0..3 {
            envelopes.push(turn_with_embedding(
                &format!("CA{i}"),
                &format!("SCA{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", i),
                vec![1.0 + (i as f64) * 0.01, 1.0 + (i as f64) * 0.02],
                &format!("ca{i}"),
            ));
        }
        for i in 0..3 {
            envelopes.push(turn_with_embedding(
                &format!("CB{i}"),
                &format!("SCB{i}"),
                "cortex",
                &format!("2026-04-26T19:{:02}:00Z", 10 + i),
                vec![10.0 + (i as f64) * 0.01, 10.0 + (i as f64) * 0.02],
                &format!("cb{i}"),
            ));
        }
        // 5 nexus turns near the cortex cluster A — must NOT show
        // up when fetching for `repo = cortex` (repo predicate
        // gates them out before clustering).
        for i in 0..5 {
            envelopes.push(turn_with_embedding(
                &format!("NX{i}"),
                &format!("SNX{i}"),
                "nexus",
                &format!("2026-04-26T19:{:02}:00Z", 20 + i),
                vec![1.0 + (i as f64) * 0.005, 1.0 + (i as f64) * 0.005],
                &format!("nx{i}"),
            ));
        }
        write_archive(dir.path(), &envelopes);
        let src = LiveTopicSource::new(dir.path(), 3);
        let got = src.fetch("cortex", SINCE_MS, UNTIL_MS).unwrap();
        assert!(!got.is_empty(), "cortex must yield ≥ 1 cluster");
        for cluster in &got {
            assert_eq!(cluster.repo, "cortex");
            for session in &cluster.sessions {
                assert!(
                    session.session_id.starts_with("SC"),
                    "leak: {} should be cortex-only",
                    session.session_id
                );
            }
        }
    }
}
