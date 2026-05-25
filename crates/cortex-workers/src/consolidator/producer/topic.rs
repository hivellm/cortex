//! Phase11j §2.5 — Topic producer.
//!
//! Input: turn-vector embeddings clustered via HDBSCAN
//! (`min_cluster_size = 3`) per repo, surfaced as a [`TopicCluster`]
//! with the constituent session ids + per-session metadata. Output:
//! one consolidation per cluster with `grain = Topic`.
//!
//! HDBSCAN clustering itself runs in the orchestrator (§2.7) so the
//! producer stays focused on the prompt + payload assembly. This
//! keeps the algorithm choice swappable (HDBSCAN today; OPTICS or
//! density-of-density tomorrow) without touching the producer.

use std::collections::BTreeMap;

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, TimeSpan,
    CONSOLIDATION_SOURCE_IDS_INLINE_CAP,
};

use super::super::summariser::{Summariser, SummariserRequest};
use super::super::templates::Template;

use super::{validate_produced, ProducedConsolidation, ProducerError};

/// Phase11j §2.5 — minimum cluster size HDBSCAN runs with.
pub const MIN_CLUSTER_SIZE: usize = 3;

/// One session's worth of metadata the topic producer needs to
/// render a useful prompt. The orchestrator builds this from the
/// archive_loader + classifier output before invoking the producer.
#[derive(Debug, Clone)]
pub struct ClusterSession {
    /// Session ULID.
    pub session_id: String,
    /// Earliest envelope `occurred_at` in epoch ms.
    pub start_ms: i64,
    /// Latest envelope `occurred_at` in epoch ms.
    pub end_ms: i64,
    /// Per-outcome counter — drives the cluster-level
    /// outcome_distribution.
    pub outcome_distribution: BTreeMap<String, u32>,
    /// One-line digest of the session (the orchestrator pulls the
    /// first user prompt + last assistant takeaway).
    pub one_line_digest: String,
}

/// One cluster the orchestrator hands the topic producer.
#[derive(Debug, Clone)]
pub struct TopicCluster {
    /// Stable label the orchestrator derives from the cluster
    /// centroid (typically a noun phrase). Drives `scope = Topic(_)`.
    pub label: String,
    /// Repo the cluster lives in. Topic clusters never cross repos.
    pub repo: String,
    /// Sessions inside the cluster, ordered by `start_ms`.
    pub sessions: Vec<ClusterSession>,
}

impl TopicCluster {
    /// Sanity check before invoking the summariser.
    pub fn ensure_min_size(&self) -> Result<(), ProducerError> {
        if self.sessions.len() < MIN_CLUSTER_SIZE {
            return Err(ProducerError::EmptyInput(format!(
                "topic cluster {:?} has {} sessions, below MIN_CLUSTER_SIZE = {}",
                self.label,
                self.sessions.len(),
                MIN_CLUSTER_SIZE
            )));
        }
        Ok(())
    }

    /// Earliest / latest `start_ms` / `end_ms` across the cluster.
    pub fn temporal_bounds_ms(&self) -> (i64, i64) {
        let lo = self.sessions.iter().map(|s| s.start_ms).min().unwrap_or(0);
        let hi = self.sessions.iter().map(|s| s.end_ms).max().unwrap_or(0);
        (lo, hi)
    }

    /// Aggregate the per-session outcome counts into a single map.
    pub fn aggregate_outcomes(&self) -> BTreeMap<String, u32> {
        let mut out: BTreeMap<String, u32> = BTreeMap::new();
        for sess in &self.sessions {
            for (k, v) in &sess.outcome_distribution {
                *out.entry(k.clone()).or_insert(0) += v;
            }
        }
        out
    }

    /// Render the topic prompt against the cluster.
    pub fn render_prompt(&self) -> String {
        let tpl = Template::for_grain(ConsolidationGrain::Topic);
        let (lo, hi) = self.temporal_bounds_ms();
        let span = format!(
            "{} → {}",
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(lo)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "—".into()),
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(hi)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "—".into()),
        );
        let outcome_summary = self
            .aggregate_outcomes()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let cluster_size = self.sessions.len().to_string();
        let source_block = self
            .sessions
            .iter()
            .map(|s| format!("- {} | {}", s.session_id, s.one_line_digest))
            .collect::<Vec<_>>()
            .join("\n");
        tpl.render([
            ("topic_label", self.label.as_str()),
            ("repo", self.repo.as_str()),
            ("cluster_size", cluster_size.as_str()),
            ("temporal_span", span.as_str()),
            ("outcome_distribution", outcome_summary.as_str()),
            ("source_sessions", source_block.as_str()),
        ])
    }
}

#[derive(Debug, serde::Deserialize)]
struct ModelResponse {
    title: String,
    summary_markdown: String,
    #[serde(default)]
    takeaways: Vec<String>,
}

fn strip_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim_start();
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

fn parse_response(raw: &str) -> Result<ModelResponse, ProducerError> {
    serde_json::from_str(strip_fence(raw)).map_err(|err| {
        ProducerError::InvalidResponse(format!(
            "topic response did not match {{title, summary_markdown, takeaways}}: {err}"
        ))
    })
}

/// Phase11j §2.5 — full pipeline.
pub async fn produce(
    cluster: &TopicCluster,
    summariser: &dyn Summariser,
) -> Result<ProducedConsolidation, ProducerError> {
    cluster.ensure_min_size()?;
    let prompt = cluster.render_prompt();
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let parsed = parse_response(&result.text)?;
    let (start_ms, end_ms) = cluster.temporal_bounds_ms();

    // Topic clusters can hold many sessions but the source-id list
    // tracks SESSIONS (not envelopes) because the cluster is the
    // producer's unit of source. The renderer derives full envelope
    // recall via the per-session consolidations downstream of this
    // grain.
    let mut source_event_ids: Vec<String> = cluster
        .sessions
        .iter()
        .map(|s| s.session_id.clone())
        .collect();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }

    let scope = ConsolidationScope::Topic(cluster.label.clone());
    let payload = ConsolidationPayload {
        consolidation_id: super::derive_consolidation_id(ConsolidationGrain::Topic, &scope),
        grain: ConsolidationGrain::Topic,
        scope,
        title: clip_title(&parsed.title),
        summary_markdown: parsed.summary_markdown,
        takeaways: parsed.takeaways,
        source_event_ids,
        source_event_count,
        model: result.kind.model_id().to_string(),
        depth: match result.kind {
            super::super::summariser::SummariserKind::Haiku45 => ConsolidationDepth::Shallow,
            super::super::summariser::SummariserKind::Opus47 => ConsolidationDepth::Deep,
        },
        outcome_distribution: cluster.aggregate_outcomes(),
        temporal_span: TimeSpan {
            start_ms,
            end_ms,
            duration_ms: (end_ms - start_ms).max(0),
        },
        repos: vec![cluster.repo.clone()],
        tags: vec![cluster.label.clone()],
    };
    validate_produced(&payload)?;
    Ok(ProducedConsolidation {
        payload,
        cost_cents: result.cost_cents,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
    })
}

fn clip_title(raw: &str) -> String {
    raw.chars()
        .take(cortex_core::events::CONSOLIDATION_TITLE_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::super::summariser::{
        SummariserError, SummariserKind, SummariserRequest as Req, SummariserResult,
    };
    use super::*;

    fn session(id: &str, start_ms: i64, end_ms: i64, outcomes: &[(&str, u32)]) -> ClusterSession {
        ClusterSession {
            session_id: id.into(),
            start_ms,
            end_ms,
            outcome_distribution: outcomes
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            one_line_digest: format!("digest for {id}"),
        }
    }

    struct StubSummariser {
        text: String,
        kind: SummariserKind,
    }
    #[async_trait::async_trait]
    impl Summariser for StubSummariser {
        fn kind(&self) -> SummariserKind {
            self.kind
        }
        async fn summarise(&self, _req: Req) -> Result<SummariserResult, SummariserError> {
            Ok(SummariserResult {
                text: self.text.clone(),
                cost_cents: 80,
                kind: self.kind,
                input_tokens: 50,
                output_tokens: 200,
            })
        }
    }

    #[test]
    fn ensure_min_size_rejects_undersized_cluster() {
        let c = TopicCluster {
            label: "hnsw".into(),
            repo: "cortex".into(),
            sessions: vec![session("a", 0, 10, &[]), session("b", 0, 10, &[])],
        };
        c.ensure_min_size().expect_err("2 < 3");
    }

    #[test]
    fn ensure_min_size_accepts_cluster_at_threshold() {
        let c = TopicCluster {
            label: "hnsw".into(),
            repo: "cortex".into(),
            sessions: vec![
                session("a", 0, 10, &[]),
                session("b", 0, 10, &[]),
                session("c", 0, 10, &[]),
            ],
        };
        c.ensure_min_size().expect("3 >= 3");
    }

    #[test]
    fn aggregate_outcomes_sums_per_session_counts() {
        let c = TopicCluster {
            label: "x".into(),
            repo: "cortex".into(),
            sessions: vec![
                session("a", 0, 10, &[("success", 3), ("error", 1)]),
                session("b", 0, 10, &[("success", 2)]),
                session("c", 0, 10, &[("error", 1)]),
            ],
        };
        let agg = c.aggregate_outcomes();
        assert_eq!(agg.get("success").copied(), Some(5));
        assert_eq!(agg.get("error").copied(), Some(2));
    }

    #[test]
    fn temporal_bounds_pick_min_start_and_max_end() {
        let c = TopicCluster {
            label: "x".into(),
            repo: "cortex".into(),
            sessions: vec![
                session("a", 100, 200, &[]),
                session("b", 50, 150, &[]),
                session("c", 80, 300, &[]),
            ],
        };
        let (lo, hi) = c.temporal_bounds_ms();
        assert_eq!(lo, 50);
        assert_eq!(hi, 300);
    }

    #[tokio::test]
    async fn produce_emits_topic_grain_with_label_in_scope_and_tags() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "HNSW recall tuning",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["ef_search 128 holds recall@10", "ef_search 64 cuts latency", "doc rewrite per quarter"]
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let cluster = TopicCluster {
            label: "hnsw recall".into(),
            repo: "cortex".into(),
            sessions: vec![
                session("01HX01", 100, 200, &[("success", 1)]),
                session("01HX02", 110, 210, &[("success", 1)]),
                session("01HX03", 120, 220, &[("error", 1)]),
            ],
        };
        let produced = produce(&cluster, &summariser).await.expect("produce");
        assert_eq!(produced.payload.grain, ConsolidationGrain::Topic);
        assert_eq!(
            produced.payload.scope,
            ConsolidationScope::Topic("hnsw recall".into())
        );
        assert_eq!(produced.payload.repos, vec!["cortex".to_string()]);
        assert_eq!(produced.payload.tags, vec!["hnsw recall".to_string()]);
        assert_eq!(produced.payload.source_event_count, 3);
        assert_eq!(
            produced
                .payload
                .outcome_distribution
                .get("success")
                .copied(),
            Some(2)
        );
    }
}
