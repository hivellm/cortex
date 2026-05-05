//! Phase11r §4.6 — end-to-end integration test for the topic-card
//! MCP tools. Each §4.X tool grows this file as it lands.
//!
//! The tests run against an in-process [`TopicCardLookup`] fake so
//! they stay hermetic — the live Vectorizer / Meili reads are
//! exercised by the routing-IT in `topic_card_routing_it.rs` (§3.6)
//! and the env-gated worker IT in `topic_cards_end_to_end_it.rs`
//! (§2.9). Pinning the MCP-side dispatch shape here is what
//! catches drift between the descriptor schema, the dispatcher, and
//! the `TopicCardLookup` contract — all three drift independently.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_api::{
    invoke_synthesize, invoke_topic_diff, invoke_topic_drill, invoke_topic_get,
    invoke_topic_neighbors, is_valid_topic_slug, synthesize_descriptor, topic_diff_descriptor,
    topic_drill_descriptor, topic_get_descriptor, topic_neighbors_descriptor, types::Scope,
    DrillDimension, HydratedEvidenceItem, MemoryAuditPublisher, NeighborEdge, NeighborGraph,
    NeighborNode, SynthesizeRequest, SynthesizeResult, TopicCardDiffer, TopicCardDrill,
    TopicCardLookup, TopicCardMcpError, TopicCardNeighbors, TopicCardRevision,
    TopicCardSynthesizer, TOOL_NAME_SYNTHESIZE, TOOL_NAME_TOPIC_DIFF, TOOL_NAME_TOPIC_DRILL,
    TOOL_NAME_TOPIC_GET, TOOL_NAME_TOPIC_NEIGHBORS, TOPIC_GET_CONFIDENCE_FLOOR,
};
use cortex_core::events::{
    derive_topic_card_id, Contradiction, ContradictionKind, ContradictionStatus, EvidenceKind,
    EvidenceRef, TopicCardPayload,
};

#[derive(Default)]
struct FakeLookup {
    by_slug: Mutex<BTreeMap<String, TopicCardPayload>>,
    search_hit: Mutex<Option<TopicCardPayload>>,
}

#[async_trait]
impl TopicCardLookup for FakeLookup {
    async fn get_by_slug(
        &self,
        slug: &str,
        _scope: &Scope,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
        Ok(self.by_slug.lock().unwrap().get(slug).cloned())
    }
    async fn search(
        &self,
        _query: &str,
        _scope: &Scope,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
        Ok(self.search_hit.lock().unwrap().clone())
    }
}

fn card(slug: &str, repo: &str, confidence: f32) -> TopicCardPayload {
    TopicCardPayload {
        topic_card_id: derive_topic_card_id(slug, repo),
        topic_slug: slug.to_string(),
        repos: vec![repo.to_string()],
        revision: 1,
        synthesis_markdown:
            "Synthesis body that exceeds the 200-byte minimum so the validator does not \
            trip if this payload is ever round-tripped through the JSON Schema. The body \
            only needs to read sensibly for the test reader; the bytes here are filler."
                .to_string(),
        evidence: Vec::new(),
        contradictions: Vec::new(),
        open_questions: Vec::new(),
        related_topic_ids: Vec::new(),
        confidence,
        last_rev_at: "2026-05-03T12:00:00Z".to_string(),
        events_since_last_rev: 0,
        synthesis_model: "claude-haiku-4-5".to_string(),
        synthesis_cost_cents: 80,
    }
}

fn scope(repo: &str) -> Scope {
    Scope {
        repo: Some(repo.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn topic_get_end_to_end_dispatches_slug_then_search_with_confidence_floor() {
    // Phase11r §4.1 — the public MCP surface must:
    //   (a) advertise `cortex_topic_get` via `topic_get_descriptor`;
    //   (b) short-circuit to `get_by_slug` when the input is a valid
    //       kebab-case slug, returning the card verbatim regardless
    //       of confidence;
    //   (c) drop through to `search` for free-text queries, applying
    //       the confidence floor (≥ 0.6) to the top-1 hit.
    let descriptor = topic_get_descriptor();
    assert_eq!(descriptor["name"], TOOL_NAME_TOPIC_GET);
    let input_schema = &descriptor["inputSchema"];
    assert_eq!(
        input_schema["required"],
        serde_json::json!(["query_or_slug", "scope"])
    );

    let lookup = Arc::new(FakeLookup::default());
    lookup.by_slug.lock().unwrap().insert(
        "auth-rewrite".to_string(),
        card("auth-rewrite", "cortex", 0.45),
    );
    *lookup.search_hit.lock().unwrap() = Some(card("query-side", "cortex", 0.72));
    let audit = MemoryAuditPublisher::new();

    // Slug-exact path — confidence below the floor is acceptable.
    assert!(is_valid_topic_slug("auth-rewrite"));
    let by_slug = invoke_topic_get(
        lookup.clone(),
        &audit,
        "claude-code",
        scope("cortex"),
        "auth-rewrite".into(),
    )
    .await
    .expect("happy path");
    let by_slug = by_slug.expect("slug-exact returns the card verbatim");
    assert_eq!(by_slug.topic_slug, "auth-rewrite");
    assert!(by_slug.confidence < TOPIC_GET_CONFIDENCE_FLOOR);

    // Free-text path — top-1 with confidence ≥ floor is returned.
    let by_query = invoke_topic_get(
        lookup.clone(),
        &audit,
        "claude-code",
        scope("cortex"),
        "how does auth work?".into(),
    )
    .await
    .expect("happy path");
    let by_query = by_query.expect("above-floor search hit returned");
    assert_eq!(by_query.topic_slug, "query-side");
    assert!(by_query.confidence >= TOPIC_GET_CONFIDENCE_FLOOR);

    // Free-text path with a below-floor hit returns None — the
    // tool prefers a hard null over noisy context.
    *lookup.search_hit.lock().unwrap() = Some(card("weak", "cortex", 0.4));
    let none = invoke_topic_get(
        lookup.clone(),
        &audit,
        "claude-code",
        scope("cortex"),
        "vague question".into(),
    )
    .await
    .expect("happy path");
    assert!(none.is_none());

    // Missing scope.repo trips the 422-equivalent error before
    // either lookup lane is touched.
    let err = invoke_topic_get(
        lookup,
        &audit,
        "claude-code",
        Scope::default(),
        "auth-rewrite".into(),
    )
    .await
    .expect_err("must reject empty scope.repo");
    assert!(matches!(err, TopicCardMcpError::ScopeRepoRequired));

    // Four invocations, four audit envelopes — every call
    // (success / miss / rejection) is recorded so the dashboard's
    // audit lane has a complete trail.
    let envs = audit.snapshot();
    assert_eq!(envs.len(), 4);
    let vias: Vec<&str> = envs
        .iter()
        .map(|e| e["result"]["via"].as_str().unwrap_or("none"))
        .collect();
    assert_eq!(vias, vec!["slug_exact", "search", "search", "none"]);
    assert_eq!(envs[3]["result"]["error"], "scope_repo_required");
}

#[derive(Default)]
struct FakeDrill {
    cards: Mutex<BTreeMap<String, TopicCardPayload>>,
    history: Mutex<BTreeMap<String, Vec<TopicCardRevision>>>,
    related: Mutex<BTreeMap<String, Vec<String>>>,
}

#[async_trait]
impl TopicCardDrill for FakeDrill {
    async fn get_card(
        &self,
        topic_card_id: &str,
    ) -> Result<Option<TopicCardPayload>, TopicCardMcpError> {
        Ok(self.cards.lock().unwrap().get(topic_card_id).cloned())
    }
    async fn hydrate_evidence(
        &self,
        evidence: &[EvidenceRef],
    ) -> Result<Vec<HydratedEvidenceItem>, TopicCardMcpError> {
        Ok(evidence
            .iter()
            .map(|e| HydratedEvidenceItem {
                kind: e.kind,
                id: e.id.clone(),
                title: format!("source-{}", e.id),
                occurred_at: "2026-04-01T00:00:00Z".to_string(),
                cited_at_rev: e.cited_at_rev,
                weight: e.weight,
            })
            .collect())
    }
    async fn history(
        &self,
        topic_card_id: &str,
    ) -> Result<Vec<TopicCardRevision>, TopicCardMcpError> {
        Ok(self
            .history
            .lock()
            .unwrap()
            .get(topic_card_id)
            .cloned()
            .unwrap_or_default())
    }
    async fn related(&self, topic_card_id: &str) -> Result<Vec<String>, TopicCardMcpError> {
        Ok(self
            .related
            .lock()
            .unwrap()
            .get(topic_card_id)
            .cloned()
            .unwrap_or_default())
    }
}

#[tokio::test]
async fn topic_drill_end_to_end_dispatches_each_dimension() {
    // Phase11r §4.2 — exercise every drill dimension end-to-end
    // through the public re-exports. One IT case covers the full
    // surface so a regression in the dispatcher (e.g. an arm
    // accidentally dropping into the wrong lane) surfaces here
    // before the unit tests do.
    let descriptor = topic_drill_descriptor();
    assert_eq!(descriptor["name"], TOOL_NAME_TOPIC_DRILL);
    let dim_enum = descriptor["inputSchema"]["properties"]["dimension"]["enum"]
        .as_array()
        .expect("dimension enum is an array");
    assert_eq!(dim_enum.len(), 5);

    let mut card = card("auth-rewrite", "cortex", 0.82);
    card.evidence = vec![EvidenceRef {
        kind: EvidenceKind::Decision,
        id: "DEC-0042".to_string(),
        weight: None,
        cited_at_rev: 3,
    }];
    card.contradictions = vec![Contradiction {
        kind: ContradictionKind::DecisionSupersession,
        evidence_a: "DEC-0042".to_string(),
        evidence_b: "DEC-0001".to_string(),
        surfaced_at_rev: 3,
        status: ContradictionStatus::Open,
    }];
    card.open_questions = vec!["why".to_string()];
    let card_id = card.topic_card_id.clone();

    let drill = Arc::new(FakeDrill::default());
    drill.cards.lock().unwrap().insert(card_id.clone(), card);
    drill.history.lock().unwrap().insert(
        card_id.clone(),
        vec![TopicCardRevision {
            topic_card_id: card_id.clone(),
            revision: 1,
            last_rev_at: "2026-05-01T00:00:00Z".to_string(),
            synthesis_diff_hash: "0".repeat(64),
        }],
    );
    drill.related.lock().unwrap().insert(
        card_id.clone(),
        vec!["topic-".to_string() + &"a".repeat(24)],
    );

    let audit = MemoryAuditPublisher::new();
    for (dim, label) in [
        (DrillDimension::Evidence, "evidence"),
        (DrillDimension::Contradictions, "contradictions"),
        (DrillDimension::History, "history"),
        (DrillDimension::OpenQuestions, "open_questions"),
        (DrillDimension::Related, "related"),
    ] {
        let out = invoke_topic_drill(drill.clone(), &audit, "claude-code", card_id.clone(), dim)
            .await
            .unwrap_or_else(|e| panic!("drill {label} failed: {e}"));
        assert_eq!(out.dimension, dim);
        assert_eq!(out.topic_card_id, card_id);
    }
    let envs = audit.snapshot();
    assert_eq!(envs.len(), 5);
    let dims: Vec<&str> = envs
        .iter()
        .map(|e| e["result"]["dimension"].as_str().unwrap())
        .collect();
    assert_eq!(
        dims,
        vec![
            "evidence",
            "contradictions",
            "history",
            "open_questions",
            "related"
        ]
    );
}

#[derive(Default)]
struct FakeNeighbors {
    graphs: Mutex<BTreeMap<String, NeighborGraph>>,
}

#[async_trait]
impl TopicCardNeighbors for FakeNeighbors {
    async fn neighbors(
        &self,
        topic_card_id: &str,
        depth: u8,
    ) -> Result<NeighborGraph, TopicCardMcpError> {
        Ok(self
            .graphs
            .lock()
            .unwrap()
            .get(topic_card_id)
            .cloned()
            .unwrap_or_else(|| NeighborGraph {
                topic_card_id: topic_card_id.to_string(),
                depth,
                nodes: Vec::new(),
                edges: Vec::new(),
                truncated: false,
            }))
    }
}

#[tokio::test]
async fn topic_neighbors_end_to_end_returns_subgraph_with_audit_envelope() {
    // Phase11r §4.3 — exercise the public re-exports + descriptor +
    // dispatcher + audit envelope. Pins the wire shape so a future
    // rename / field drop trips the IT before unit tests catch it.
    let descriptor = topic_neighbors_descriptor();
    assert_eq!(descriptor["name"], TOOL_NAME_TOPIC_NEIGHBORS);
    let depth_schema = &descriptor["inputSchema"]["properties"]["depth"];
    assert_eq!(depth_schema["minimum"], 1);
    assert_eq!(depth_schema["maximum"], 5);

    let root = "topic-".to_string() + &"a".repeat(24);
    let leaf = "topic-".to_string() + &"b".repeat(24);
    let neighbors = Arc::new(FakeNeighbors::default());
    neighbors.graphs.lock().unwrap().insert(
        root.clone(),
        NeighborGraph {
            topic_card_id: root.clone(),
            depth: 2,
            nodes: vec![
                NeighborNode {
                    topic_card_id: root.clone(),
                    topic_slug: "auth-rewrite".to_string(),
                    revision: 3,
                },
                NeighborNode {
                    topic_card_id: leaf.clone(),
                    topic_slug: "session-store".to_string(),
                    revision: 1,
                },
            ],
            edges: vec![NeighborEdge {
                edge_type: "RELATED_TO".to_string(),
                from: root.clone(),
                to: leaf.clone(),
            }],
            truncated: false,
        },
    );

    let audit = MemoryAuditPublisher::new();
    let out = invoke_topic_neighbors(neighbors, &audit, "claude-code", root.clone(), Some(2))
        .await
        .expect("neighbours ok");
    assert_eq!(out.topic_card_id, root);
    assert_eq!(out.nodes.len(), 2);
    assert_eq!(out.edges[0].edge_type, "RELATED_TO");

    let envs = audit.snapshot();
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0]["tool"], TOOL_NAME_TOPIC_NEIGHBORS);
    assert_eq!(envs[0]["result"]["nodes"], 2);
    assert_eq!(envs[0]["result"]["edges"], 1);
}

#[derive(Default)]
struct FakeDiffer {
    pairs: Mutex<BTreeMap<(String, u32), (TopicCardPayload, TopicCardPayload)>>,
}

#[async_trait]
impl TopicCardDiffer for FakeDiffer {
    async fn revision_pair(
        &self,
        topic_card_id: &str,
        since_rev: u32,
    ) -> Result<Option<(TopicCardPayload, TopicCardPayload)>, TopicCardMcpError> {
        Ok(self
            .pairs
            .lock()
            .unwrap()
            .get(&(topic_card_id.to_string(), since_rev))
            .cloned())
    }
}

#[tokio::test]
async fn topic_diff_end_to_end_renders_synthesis_and_set_diffs() {
    let descriptor = topic_diff_descriptor();
    assert_eq!(descriptor["name"], TOOL_NAME_TOPIC_DIFF);
    assert_eq!(
        descriptor["inputSchema"]["required"],
        serde_json::json!(["topic_card_id", "since_rev"])
    );

    let card_id = derive_topic_card_id("auth-rewrite", "cortex");
    let mut from = card("auth-rewrite", "cortex", 0.7);
    from.revision = 1;
    from.synthesis_markdown = "Intro\nMiddle old\nOutro".to_string() + &" filler".repeat(40); // ≥ 200 bytes for schema parity
    from.evidence = vec![EvidenceRef {
        kind: EvidenceKind::Decision,
        id: "DEC-0001".to_string(),
        weight: None,
        cited_at_rev: 1,
    }];

    let mut to = card("auth-rewrite", "cortex", 0.85);
    to.revision = 3;
    to.synthesis_markdown = "Intro\nMiddle new\nOutro".to_string() + &" filler".repeat(40);
    to.evidence = vec![
        EvidenceRef {
            kind: EvidenceKind::Decision,
            id: "DEC-0001".to_string(),
            weight: None,
            cited_at_rev: 3,
        },
        EvidenceRef {
            kind: EvidenceKind::Law,
            id: "LAW-CORTEX-001".to_string(),
            weight: None,
            cited_at_rev: 3,
        },
    ];

    let differ = Arc::new(FakeDiffer::default());
    differ
        .pairs
        .lock()
        .unwrap()
        .insert((card_id.clone(), 1), (from, to));

    let audit = MemoryAuditPublisher::new();
    let diff = invoke_topic_diff(differ, &audit, "claude-code", card_id.clone(), 1)
        .await
        .expect("diff ok");
    assert_eq!(diff.from_rev, 1);
    assert_eq!(diff.to_rev, 3);
    assert_eq!(diff.evidence_added.len(), 1);
    assert_eq!(diff.evidence_added[0].id, "LAW-CORTEX-001");
    assert!(diff.synthesis_diff.contains("- Middle old"));
    assert!(diff.synthesis_diff.contains("+ Middle new"));

    let envs = audit.snapshot();
    assert_eq!(envs[0]["tool"], TOOL_NAME_TOPIC_DIFF);
    assert_eq!(envs[0]["result"]["from_rev"], 1);
    assert_eq!(envs[0]["result"]["to_rev"], 3);
}

#[derive(Default)]
struct FakeSynthesizer {
    persisted_calls: Mutex<Vec<bool>>,
    cap_cents: Mutex<u32>,
    used_cents: Mutex<u32>,
}

#[async_trait]
impl TopicCardSynthesizer for FakeSynthesizer {
    async fn synthesize(
        &self,
        req: SynthesizeRequest,
    ) -> Result<SynthesizeResult, TopicCardMcpError> {
        let used = *self.used_cents.lock().unwrap();
        let cap = *self.cap_cents.lock().unwrap();
        if used >= cap {
            return Err(TopicCardMcpError::BudgetExhausted {
                used_cents: used,
                cap_cents: cap,
            });
        }
        self.persisted_calls.lock().unwrap().push(req.persist);
        let mut produced = card("auth-rewrite", "cortex", 0.78);
        produced.synthesis_markdown = "synthesised body padded so the validator does not \
            trip — body needs to read sensibly for the test reader and exceed 200 bytes."
            .to_string();
        Ok(SynthesizeResult {
            topic_card: produced,
            cost_cents: 100,
            persisted: req.persist,
        })
    }
}

#[tokio::test]
async fn topic_synthesize_end_to_end_runs_persist_paths_and_budget_gate() {
    // Phase11r §4.5 — exercise persist=false, persist=true, and the
    // BudgetExhausted rejection through the public re-exports.
    let descriptor = synthesize_descriptor();
    assert_eq!(descriptor["name"], TOOL_NAME_SYNTHESIZE);
    assert_eq!(
        descriptor["inputSchema"]["required"],
        serde_json::json!(["query", "scope"])
    );

    let synth = Arc::new(FakeSynthesizer::default());
    *synth.cap_cents.lock().unwrap() = 10_000;
    *synth.used_cents.lock().unwrap() = 100;

    let audit = MemoryAuditPublisher::new();
    let preview = invoke_synthesize(
        synth.clone(),
        &audit,
        "claude-code",
        SynthesizeRequest {
            query: "auth rewrite".to_string(),
            scope: scope("cortex"),
            force: false,
            persist: false,
        },
    )
    .await
    .expect("synth ok");
    assert!(!preview.persisted);

    let persisted = invoke_synthesize(
        synth.clone(),
        &audit,
        "claude-code",
        SynthesizeRequest {
            query: "auth rewrite".to_string(),
            scope: scope("cortex"),
            force: true,
            persist: true,
        },
    )
    .await
    .expect("synth ok");
    assert!(persisted.persisted);

    // Cap → used so the next call trips the budget.
    *synth.used_cents.lock().unwrap() = 10_000;
    let err = invoke_synthesize(
        synth,
        &audit,
        "claude-code",
        SynthesizeRequest {
            query: "auth rewrite".to_string(),
            scope: scope("cortex"),
            force: false,
            persist: false,
        },
    )
    .await
    .expect_err("over-cap must reject");
    assert!(matches!(err, TopicCardMcpError::BudgetExhausted { .. }));

    // Three invocations → three audit envelopes (preview, persist, reject).
    let envs = audit.snapshot();
    assert_eq!(envs.len(), 3);
    assert_eq!(envs[0]["result"]["persisted"], false);
    assert_eq!(envs[1]["result"]["persisted"], true);
    assert_eq!(envs[2]["result"]["error"], "budget_exhausted");
}

// -----------------------------------------------------------------------
// §4.6 — additional edge cases consolidating coverage across §4 tools.
// -----------------------------------------------------------------------

#[tokio::test]
async fn topic_get_returns_none_when_no_card_matches_either_lane() {
    // Phase11r §4.6 edge case "topic not found" — neither slug
    // lane nor search lane resolves; tool returns Ok(None) with a
    // populated audit envelope so the dashboard sees the miss.
    let lookup = Arc::new(FakeLookup::default());
    let audit = MemoryAuditPublisher::new();
    let result = invoke_topic_get(
        lookup,
        &audit,
        "claude-code",
        scope("cortex"),
        "completely-unknown".into(),
    )
    .await
    .expect("happy path");
    assert!(result.is_none());
    let envs = audit.snapshot();
    assert_eq!(envs[0]["result"]["hit"], serde_json::Value::Null);
}

#[tokio::test]
async fn topic_drill_returns_invalid_when_card_missing() {
    // §4.6 edge case "drill on missing card" — the tool returns
    // TopicCardMcpError::Invalid with a "not found" message rather
    // than panicking or returning empty data.
    let drill = Arc::new(FakeDrill::default());
    let audit = MemoryAuditPublisher::new();
    let err = invoke_topic_drill(
        drill,
        &audit,
        "claude-code",
        "topic-".to_string() + &"f".repeat(24),
        DrillDimension::Evidence,
    )
    .await
    .expect_err("missing card must error");
    match err {
        TopicCardMcpError::Invalid(msg) => assert!(msg.contains("not found")),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn topic_neighbors_clamps_depth_zero_to_one() {
    // §4.6 edge case "neighbors at depth=0" — JSON Schema rejects
    // depth=0 upstream when the runtime validates, but the
    // dispatcher's clamp handles bypassed validation. Pin the
    // contract.
    let neighbors = Arc::new(FakeNeighbors::default());
    let audit = MemoryAuditPublisher::new();
    let root = "topic-".to_string() + &"a".repeat(24);
    let out = invoke_topic_neighbors(neighbors, &audit, "claude-code", root, Some(0))
        .await
        .expect("neighbours ok");
    // The fake's default empty graph carries the clamped depth.
    assert_eq!(out.depth, 1);
    assert_eq!(audit.snapshot()[0]["result"]["depth"], 1);
}

#[tokio::test]
async fn topic_diff_rejects_future_since_rev() {
    // §4.6 edge case "diff against a future rev" — `since_rev` ≥
    // current rev returns Invalid("must be older").
    let card_id = derive_topic_card_id("auth-rewrite", "cortex");
    let mut from = card("auth-rewrite", "cortex", 0.7);
    from.revision = 5;
    let mut to = card("auth-rewrite", "cortex", 0.7);
    to.revision = 5;
    let differ = Arc::new(FakeDiffer::default());
    differ
        .pairs
        .lock()
        .unwrap()
        .insert((card_id.clone(), 5), (from, to));
    let audit = MemoryAuditPublisher::new();
    let err = invoke_topic_diff(differ, &audit, "claude-code", card_id, 5)
        .await
        .expect_err("future since_rev must reject");
    match err {
        TopicCardMcpError::Invalid(msg) => assert!(msg.contains("must be older")),
        other => panic!("expected Invalid, got {other:?}"),
    }
}
