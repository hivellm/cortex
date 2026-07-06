//! Phase27c §1 — Community producer.
//!
//! Input: one graph community from the phase27b Leiden partition —
//! its member nodes, god nodes, and cross-community edges — surfaced
//! as a [`CommunityInput`] at one hierarchy level. Output: one
//! consolidation with `grain = Community` whose scope carries
//! `{community_id, level}` (§1.2), so multi-resolution summaries
//! (§1.3) are simply the same producer run once per Leiden level.
//!
//! The Nexus snapshot that builds the input lives in the source
//! selector (`source/community.rs`), mirroring how HDBSCAN lives in
//! the orchestrator for the Topic grain: the producer stays focused
//! on prompt + payload assembly, so the snapshot query can evolve
//! (or be replaced by a test fixture) without touching this file.

use std::collections::BTreeMap;

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, TimeSpan,
    CONSOLIDATION_SOURCE_IDS_INLINE_CAP,
};

use super::super::summariser::{Summariser, SummariserRequest};
use super::super::templates::Template;

use super::{validate_produced, ProducedConsolidation, ProducerError};

/// Phase27c §1 — minimum member count worth an LLM call. A 1-2 node
/// "community" carries no subsystem story; skip it.
pub const MIN_COMMUNITY_SIZE: usize = 3;

/// One member node of the community, as the source selector
/// projected it from the Nexus snapshot.
#[derive(Debug, Clone)]
pub struct CommunityMember {
    /// Graph node id (`_id`, Nexus reserved slot).
    pub id: String,
    /// Node label (`Symbol`, `Artifact`, …).
    pub label: String,
    /// Display name (falls back to `id` upstream when absent).
    pub name: String,
    /// God-node flag (`is_god_node` property from the phase27b
    /// writeback) — hubs excluded from the partition then
    /// re-attached by neighbor majority.
    pub is_god_node: bool,
}

/// One edge leaving this community for another.
#[derive(Debug, Clone)]
pub struct CommunityCrossEdge {
    /// Source node id (inside this community).
    pub from: String,
    /// Target node id (in `other_community`).
    pub to: String,
    /// Relationship type (`CALLS`, `IMPORTS`, …).
    pub relation: String,
    /// The neighboring community's id.
    pub other_community: u32,
}

/// Everything the community producer needs for one summary run.
#[derive(Debug, Clone)]
pub struct CommunityInput {
    /// Partition id at `level`.
    pub community_id: u32,
    /// Leiden hierarchy level (0 = coarsest subsystem cut).
    pub level: u32,
    /// Repo the snapshot was scoped to.
    pub repo: String,
    /// Member nodes (god nodes included, flagged).
    pub members: Vec<CommunityMember>,
    /// Edges leaving this community.
    pub cross_edges: Vec<CommunityCrossEdge>,
    /// Epoch ms the Nexus snapshot was taken at. Communities are
    /// structural, not temporal — the payload's `temporal_span`
    /// collapses to this instant (`start == end`, duration 0).
    pub snapshot_ms: i64,
}

impl CommunityInput {
    /// Sanity check before invoking the summariser.
    pub fn ensure_min_size(&self) -> Result<(), ProducerError> {
        if self.members.len() < MIN_COMMUNITY_SIZE {
            return Err(ProducerError::EmptyInput(format!(
                "community {}@L{} has {} members, below MIN_COMMUNITY_SIZE = {}",
                self.community_id,
                self.level,
                self.members.len(),
                MIN_COMMUNITY_SIZE
            )));
        }
        Ok(())
    }

    /// God-node display names, sorted for prompt determinism.
    pub fn god_node_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .members
            .iter()
            .filter(|m| m.is_god_node)
            .map(|m| m.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// Render the community prompt.
    pub fn render_prompt(&self) -> String {
        let tpl = Template::for_grain(ConsolidationGrain::Community);
        let community_id = self.community_id.to_string();
        let level = self.level.to_string();
        let member_count = self.members.len().to_string();
        let god_nodes = {
            let names = self.god_node_names();
            if names.is_empty() {
                "(none)".to_string()
            } else {
                names.join(", ")
            }
        };
        let cross_edges = if self.cross_edges.is_empty() {
            "(none)".to_string()
        } else {
            self.cross_edges
                .iter()
                .map(|e| {
                    format!(
                        "- {} -[{}]-> {} (community {})",
                        e.from, e.relation, e.to, e.other_community
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let members = self
            .members
            .iter()
            .map(|m| {
                let hub = if m.is_god_node { " [god node]" } else { "" };
                format!("{} {} {}{hub}", m.label, m.id, m.name)
            })
            .collect::<Vec<_>>()
            .join("\n");
        tpl.render([
            ("community_id", community_id.as_str()),
            ("level", level.as_str()),
            ("repo", self.repo.as_str()),
            ("member_count", member_count.as_str()),
            ("god_nodes", god_nodes.as_str()),
            ("cross_community_edges", cross_edges.as_str()),
            ("members", members.as_str()),
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
            "community response did not match {{title, summary_markdown, takeaways}}: {err}"
        ))
    })
}

/// Phase27c §1.2 — full pipeline: prompt → summariser → validated
/// `grain = Community` payload carrying `{community_id, level}` in
/// its scope.
pub async fn produce(
    input: &CommunityInput,
    summariser: &dyn Summariser,
) -> Result<ProducedConsolidation, ProducerError> {
    input.ensure_min_size()?;
    let prompt = input.render_prompt();
    let result = summariser
        .summarise(SummariserRequest {
            prompt,
            max_output_tokens: None,
        })
        .await?;
    let parsed = parse_response(&result.text)?;

    // The community's "sources" are graph node ids, not envelope
    // ULIDs — same spirit as the Topic grain tracking session ids:
    // the unit the producer consumed is the member node. The reader
    // resolves nodes → underlying artifacts via the graph.
    let mut source_event_ids: Vec<String> = input.members.iter().map(|m| m.id.clone()).collect();
    let source_event_count = source_event_ids.len() as u32;
    if source_event_ids.len() > CONSOLIDATION_SOURCE_IDS_INLINE_CAP {
        source_event_ids.truncate(CONSOLIDATION_SOURCE_IDS_INLINE_CAP);
    }

    let scope = ConsolidationScope::Community {
        community_id: input.community_id,
        level: input.level,
    };
    let payload = ConsolidationPayload {
        consolidation_id: super::derive_consolidation_id(ConsolidationGrain::Community, &scope),
        grain: ConsolidationGrain::Community,
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
        outcome_distribution: BTreeMap::new(),
        temporal_span: TimeSpan {
            start_ms: input.snapshot_ms,
            end_ms: input.snapshot_ms,
            duration_ms: 0,
        },
        repos: vec![input.repo.clone()],
        tags: vec![format!("community:{}@{}", input.community_id, input.level)],
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

    fn member(id: &str, name: &str, god: bool) -> CommunityMember {
        CommunityMember {
            id: id.into(),
            label: "Symbol".into(),
            name: name.into(),
            is_god_node: god,
        }
    }

    fn input(members: Vec<CommunityMember>) -> CommunityInput {
        CommunityInput {
            community_id: 7,
            level: 1,
            repo: "cortex".into(),
            members,
            cross_edges: vec![CommunityCrossEdge {
                from: "n1".into(),
                to: "m9".into(),
                relation: "CALLS".into(),
                other_community: 2,
            }],
            snapshot_ms: 1_000,
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
    fn ensure_min_size_rejects_undersized_community() {
        let i = input(vec![member("n1", "a", false), member("n2", "b", false)]);
        i.ensure_min_size().expect_err("2 < 3");
    }

    #[test]
    fn render_prompt_carries_members_god_nodes_and_cross_edges() {
        let i = input(vec![
            member("n1", "graph_worker", false),
            member("n2", "nexus_client", true),
            member("n3", "projection", false),
        ]);
        let prompt = i.render_prompt();
        assert!(prompt.contains("`7`"), "community_id rendered");
        assert!(prompt.contains("`1`"), "level rendered");
        assert!(prompt.contains("nexus_client"), "god node name rendered");
        assert!(
            prompt.contains("[god node]"),
            "god-node flag rendered on member line"
        );
        assert!(
            prompt.contains("n1 -[CALLS]-> m9 (community 2)"),
            "cross edge rendered"
        );
        assert!(!prompt.contains("{{"), "no unfilled template slots");
    }

    #[tokio::test]
    async fn produce_emits_community_grain_with_structured_scope() {
        let body = serde_json::to_string(&serde_json::json!({
            "title": "Graph write pipeline",
            "summary_markdown": "x".repeat(400),
            "takeaways": ["nexus_client anchors writes", "projection feeds it", "calls out to community 2", "3 members"]
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let i = input(vec![
            member("n1", "graph_worker", false),
            member("n2", "nexus_client", true),
            member("n3", "projection", false),
        ]);
        let produced = produce(&i, &summariser).await.expect("produce");
        assert_eq!(produced.payload.grain, ConsolidationGrain::Community);
        assert_eq!(
            produced.payload.scope,
            ConsolidationScope::Community {
                community_id: 7,
                level: 1
            }
        );
        assert_eq!(produced.payload.source_event_count, 3);
        assert_eq!(produced.payload.repos, vec!["cortex".to_string()]);
        assert_eq!(
            produced.payload.tags,
            vec!["community:7@1".to_string()],
            "tag carries community id + level"
        );
        assert_eq!(produced.payload.temporal_span.duration_ms, 0);
        assert!(produced.payload.consolidation_id.starts_with("cons-com-"));
    }

    #[tokio::test]
    async fn produce_is_stable_across_reruns_same_scope_same_id() {
        // §1.2 — re-running the producer for the same (community,
        // level) must land on the same consolidation_id so the
        // existing storage upserts instead of duplicating.
        let body = serde_json::to_string(&serde_json::json!({
            "title": "t",
            "summary_markdown": "x".repeat(300),
            "takeaways": []
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let i = input(vec![
            member("n1", "a", false),
            member("n2", "b", false),
            member("n3", "c", false),
        ]);
        let one = produce(&i, &summariser).await.expect("produce 1");
        let two = produce(&i, &summariser).await.expect("produce 2");
        assert_eq!(one.payload.consolidation_id, two.payload.consolidation_id);
    }

    #[tokio::test]
    async fn produce_multi_resolution_levels_get_distinct_ids() {
        // §1.3 — the same community summarised at two Leiden levels
        // is two distinct consolidations.
        let body = serde_json::to_string(&serde_json::json!({
            "title": "t",
            "summary_markdown": "x".repeat(300),
            "takeaways": []
        }))
        .unwrap();
        let summariser = StubSummariser {
            text: body,
            kind: SummariserKind::Haiku45,
        };
        let coarse = input(vec![
            member("n1", "a", false),
            member("n2", "b", false),
            member("n3", "c", false),
        ]);
        let mut fine = coarse.clone();
        fine.level = 2;
        let a = produce(&coarse, &summariser).await.expect("coarse");
        let b = produce(&fine, &summariser).await.expect("fine");
        assert_ne!(a.payload.consolidation_id, b.payload.consolidation_id);
        assert_eq!(
            b.payload.scope,
            ConsolidationScope::Community {
                community_id: 7,
                level: 2
            }
        );
    }
}
