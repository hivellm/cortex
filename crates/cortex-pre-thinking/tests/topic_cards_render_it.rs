//! Phase11r §5.5 — integration tests for the topic-card section
//! end-to-end through `format_bundle` + `clip_to_budget`. Exercises:
//!
//! 1. Section ordering (laws → topic_cards → consolidations →
//!    decisions → similar_turns → past_sessions → snippets).
//! 2. Stale-topic-card advisory + section downgrade.
//! 3. Budget contract — the 32 KiB ceiling the budget clipper
//!    targets stays inviolate even when the topic card section
//!    inflates.
//! 4. Fallback to consolidations when zero topic cards match.
//!
//! These cases run against the public `cortex_pre_thinking` re-exports
//! so a future rename of the formatter helper trips this IT before
//! the unit tests do.

use cortex_api::{
    BudgetReport, ConsolidationRef, DebugInfo, DecisionRef, GraphNeighbor, LaneTimings, LawRef,
    QueryResponse, ResultsBag, Scope, SimilarTurn, Snippet, TopicCardRef, ViolationRef,
};
use cortex_core::events::{
    Contradiction, ContradictionKind, ContradictionStatus, EvidenceKind, EvidenceRef,
};
use cortex_pre_thinking::{clip_to_budget, format_bundle, FormatOptions};

fn fresh_card() -> TopicCardRef {
    TopicCardRef {
        topic_card_id: "topic-".to_string() + &"a".repeat(24),
        topic_slug: "auth-rewrite".to_string(),
        revision: 3,
        synthesis_preview:
            "JWT validation now consolidated behind a single middleware so token rotation \
            lands deterministically without the prior 5-minute cache lag."
                .to_string(),
        evidence_top5: vec![EvidenceRef {
            kind: EvidenceKind::Decision,
            id: "DEC-0042".to_string(),
            weight: Some(0.9),
            cited_at_rev: 3,
        }],
        open_contradictions: vec![Contradiction {
            kind: ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-0042".to_string(),
            evidence_b: "DEC-0001".to_string(),
            surfaced_at_rev: 3,
            status: ContradictionStatus::Open,
        }],
        confidence: 0.82,
        synthesis_age_d: 5,
        events_since_last_rev: 2,
        score: 0.91,
    }
}

fn populated_response_with_topic(card: Option<TopicCardRef>) -> QueryResponse {
    QueryResponse {
        intent: "pre_change_context".into(),
        query_id: "01HFIT".into(),
        scope_resolved: Scope::default(),
        results: ResultsBag {
            snippets: vec![Snippet {
                rank: 1,
                source: "vector".into(),
                collection: None,
                repo: Some("Cortex".into()),
                path: Some("src/lib.rs".into()),
                symbol: Some("foo".into()),
                content_hash: None,
                text: "pub fn foo() {}".into(),
                body_truncated: false,
                score: 0.9,
                why: Some("vector match".into()),
                verified: None,
                verdict: None,
                class_level: None,
                class_compartments: vec![],
            }],
            decisions: vec![DecisionRef {
                rank: 1,
                id: "DEC-0042".into(),
                title: "Adopt Meili".into(),
                rationale_excerpt: None,
                status: "accepted".into(),
                ts: 1_715_000_000_000,
                score: 0.7,
                links: vec![],
            }],
            violations: vec![ViolationRef {
                id: "VIO-1".into(),
                law_id: "LAW-007".into(),
                severity: "critical".into(),
                message: "no --no-verify".into(),
                observed_in: "turn:01HX".into(),
            }],
            graph_neighbors: vec![GraphNeighbor {
                from: "ToolCall:01H".into(),
                relation: "TOUCHED".into(),
                to: "Artifact:V|src/lib.rs|sha".into(),
                hops: 1,
            }],
            similar_turns: vec![SimilarTurn {
                turn_id: "01HXZ".into(),
                ts: 1_715_000_000_000,
                model: "claude-sonnet".into(),
                summary: "refactored hnsw_search".into(),
                score: 0.6,
                outcome: Some("success".into()),
            }],
            past_sessions: Vec::new(),
            consolidations: vec![ConsolidationRef {
                consolidation_id: "cons-ses-deadbeef".into(),
                grain: "session".into(),
                ts: 1_715_000_000_000,
                title: "Older session".into(),
                outcome: Some("success".into()),
                score: 0.8,
            }],
            topic_cards: card.into_iter().collect(),
        },
        laws_active: vec![LawRef {
            id: "LAW-007".into(),
            severity: "critical".into(),
            title: "No --no-verify".into(),
        }],
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: 500,
            cache: "miss".into(),
        },
        debug: DebugInfo {
            lanes: LaneTimings::default(),
            errors: Default::default(),
            truncated: false,
            notes: Vec::new(),
        },
        notice: None,
        clipped: None,
    }
}

#[test]
fn section_order_with_fresh_topic_card_matches_spec() {
    // §5.4 spec order: laws → topic_cards → consolidations →
    // decisions → similar_turns → snippets.
    let resp = populated_response_with_topic(Some(fresh_card()));
    let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
    let laws = bundle.find("Active laws").unwrap();
    let topic = bundle.find("## Topic card").unwrap();
    let cons = bundle.find("Consolidated context").unwrap();
    let dec = bundle.find("Recent decisions").unwrap();
    let turns = bundle.find("Similar past turns").unwrap();
    let snippets = bundle.find("Relevant snippets").unwrap();
    assert!(laws < topic);
    assert!(topic < cons);
    assert!(cons < dec);
    assert!(dec < turns);
    assert!(turns < snippets);
}

#[test]
fn section_order_with_stale_topic_card_demotes_topic_after_consolidations() {
    // §5.4 staleness: consolidations precede topic cards when the
    // card trips the floor / age heuristic.
    let mut card = fresh_card();
    card.confidence = 0.4; // below 0.6 floor.
    let resp = populated_response_with_topic(Some(card));
    let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
    let cons = bundle.find("Consolidated context").unwrap();
    let topic = bundle.find("## Topic card").unwrap();
    assert!(cons < topic);
    assert!(bundle.contains("stale-topic-card"));
}

#[test]
fn section_order_with_age_plus_events_staleness_branch() {
    // The other staleness branch: age > 30 AND events_since > 0.
    let mut card = fresh_card();
    card.synthesis_age_d = 60;
    card.events_since_last_rev = 4;
    card.confidence = 0.95; // confidence stays above floor — only age trips
    let resp = populated_response_with_topic(Some(card));
    let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
    let cons = bundle.find("Consolidated context").unwrap();
    let topic = bundle.find("## Topic card").unwrap();
    assert!(cons < topic);
    assert!(bundle.contains("stale-topic-card"));
}

#[test]
fn no_topic_card_falls_back_to_consolidations_lane() {
    // Zero topic cards → existing consolidation lane runs intact;
    // no advisory line; section header absent.
    let resp = populated_response_with_topic(None);
    let bundle = format_bundle("pre_change_context", &resp, &FormatOptions::default());
    assert!(!bundle.contains("## Topic card"));
    assert!(!bundle.contains("stale-topic-card"));
    assert!(bundle.contains("Consolidated context"));
}

#[test]
fn topic_card_section_respects_section_byte_cap() {
    // The synthesis preview is clipped to ~half the section byte
    // cap (the formatter splits the budget between preview and
    // evidence). A 50-byte cap forces a tight clip; the bundle
    // must NOT contain the full body.
    let card = fresh_card();
    let opts = FormatOptions {
        topic_cards_byte_cap: 50,
        ..Default::default()
    };
    let resp = populated_response_with_topic(Some(card.clone()));
    let bundle = format_bundle("pre_change_context", &resp, &opts);
    // Header line still lands.
    assert!(bundle.contains("[auth-rewrite] (rev 3"));
    // Full body does NOT (the byte cap clipped the preview).
    assert!(!bundle.contains("5-minute cache lag"));
}

#[test]
fn budget_clipper_keeps_topic_card_bundle_under_thirty_two_kib() {
    // §5.5 — the 32 KiB budget contract. Even with a topic card
    // pushing the bundle larger than the per-section cap, the
    // budget clipper trims the response to fit. Pin the contract
    // so a future formatter change cannot blow past the ceiling.
    let mut card = fresh_card();
    card.synthesis_preview = "x".repeat(20_000); // big preview
    let resp = populated_response_with_topic(Some(card));
    let clipped = clip_to_budget("pre_change_context", &resp, 32 * 1024);
    assert!(
        clipped.bundle.len() <= 32 * 1024,
        "bundle must stay under 32 KiB; got {} bytes",
        clipped.bundle.len()
    );
}
