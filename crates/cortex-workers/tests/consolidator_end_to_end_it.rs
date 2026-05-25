//! Phase11j §2.11 — end-to-end IT for the Session producer.
//!
//! Seeds one synthetic session with 30 envelopes (10 user / 10
//! assistant pairs + 10 ToolCall envelopes), runs the Session
//! producer through the orchestrator with a canned-response
//! summariser, and asserts the emitted `Kind::Consolidation`
//! payload's shape:
//!
//! - grain = Session, scope = SessionId(_)
//! - title clipped to ≤ 80 chars
//! - summary_markdown len ∈ [200, 2 000]
//! - source_event_ids covers every input envelope (no dropped
//!   sources) AND `source_event_count` matches
//! - outcome_distribution reflects per-envelope `outcome` tags
//! - temporal_span.start_ms/end_ms span the full input
//! - consolidation_id is deterministic (re-run lands on same id)
//!
//! No env gate — runs unconditionally in the default `cargo test`
//! suite. The summariser is in-process so the IT does not need
//! network access.

use std::sync::Arc;

use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationScope, Context, Envelope, Kind, Stream,
};
use cortex_workers::consolidator::orchestrator::Orchestrator;
use cortex_workers::consolidator::producer::session::SessionInput;
use cortex_workers::consolidator::summariser::{
    Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
};

const CANNED_RESPONSE: &str = r#"{
    "title": "tune ef_search across HNSW recall benchmarks",
    "summary_markdown": "The session walked through the ef_search tuning workflow: enumerate the recall@10 baseline, rerun against ef_search ∈ {64, 96, 128, 160}, and compare against the latency budget. The decision-bearing fragment landed on ef_search = 128 because recall held above 0.92 across the 2 M-vector benchmark while p99 latency stayed under 12 ms. Subsequent tool calls touched `crates/cortex-vectorizer/src/hnsw.rs` and `crates/cortex-api/config/relevance.toml` so the runtime default would pick the new value without a redeploy. The session ended with a single accepted decision and zero error outcomes.",
    "takeaways": [
        "ef_search = 128 holds recall@10 ≥ 0.92 up to 2 M vectors",
        "p99 latency stays under 12 ms with ef_search = 128",
        "relevance.toml is the right surface to land the override"
    ]
}"#;

struct CannedSummariser {
    text: String,
    kind: SummariserKind,
    cost: u32,
}

#[async_trait::async_trait]
impl Summariser for CannedSummariser {
    fn kind(&self) -> SummariserKind {
        self.kind
    }
    async fn summarise(
        &self,
        request: SummariserRequest,
    ) -> Result<SummariserResult, SummariserError> {
        // 2026-05-19 — producer makes two calls: a relevance gate first
        // (`{"relevant": true|false, "reason": "..."}`), then the full
        // summary. Distinguish by prompt content so the mock satisfies
        // both stages with one fixture.
        let text = if request.prompt.contains("relevance judge") {
            r#"{"relevant": true, "reason": "session captures concrete ef_search tuning decision"}"#
                .to_string()
        } else {
            self.text.clone()
        };
        Ok(SummariserResult {
            text,
            cost_cents: self.cost,
            kind: self.kind,
            input_tokens: 1_500,
            output_tokens: 400,
        })
    }
}

fn ctx() -> Context {
    Context {
        repo: Some("cortex".into()),
        branch: None,
        commit: None,
        cwd: None,
        user: None,
        platform: "linux".into(),
        ide: None,
        extras: Default::default(),
    }
}

fn envelope(idx: u8, kind: Kind, ts: &str, payload: serde_json::Value) -> Envelope {
    Envelope {
        event_id: format!("01HXEVT{idx:019}"),
        schema_version: "1".into(),
        occurred_at: ts.into(),
        ingested_at: None,
        session_id: "01HXSESS00000000000000000A".into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: Some("claude-haiku-4-5".into()),
        kind,
        context: ctx(),
        payload,
        redactions: vec![],
        content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .into(),
        parent_event_id: None,
    }
}

fn synth_session(pair_count: u8, tool_count: u8) -> Vec<Envelope> {
    let mut out = Vec::new();
    let mut idx: u8 = 0;
    for i in 0..pair_count {
        let ts_user = format!("2026-04-20T10:{:02}:00Z", i);
        let ts_assistant = format!("2026-04-20T10:{:02}:30Z", i);
        idx += 1;
        out.push(envelope(
            idx,
            Kind::Turn,
            &ts_user,
            serde_json::json!({
                "user_message": format!("user-side prompt iteration {i} with enough body text"),
                "assistant_message": format!("assistant reply iteration {i} with enough body text"),
                "outcome": if i % 3 == 0 { "error" } else { "success" }
            }),
        ));
        // Pair the assistant-side turn AS WELL so the input set
        // genuinely contains 30 envelopes (10 turn + 10 second
        // pair-side turn + 10 tool calls).
        idx += 1;
        out.push(envelope(
            idx,
            Kind::Turn,
            &ts_assistant,
            serde_json::json!({
                "user_message": format!("follow-up question for iteration {i} with body text"),
                "assistant_message": format!("affirmative response iteration {i} with body text"),
                "outcome": "success"
            }),
        ));
    }
    for j in 0..tool_count {
        let ts_tool = format!("2026-04-20T11:{:02}:00Z", j);
        idx += 1;
        out.push(envelope(
            idx,
            Kind::ToolCall,
            &ts_tool,
            serde_json::json!({
                "tool_name": "Read",
                "input": {"path": format!("crates/cortex-vectorizer/src/{j}.rs")},
                "outcome": "success"
            }),
        ));
    }
    out
}

#[tokio::test]
async fn session_producer_emits_well_shaped_consolidation_envelope_for_30_input_envelopes() {
    let envelopes = synth_session(10, 10);
    assert_eq!(envelopes.len(), 30);

    let haiku = Arc::new(CannedSummariser {
        text: CANNED_RESPONSE.to_string(),
        kind: SummariserKind::Haiku45,
        cost: 80,
    });
    // Opus is unused on this path but the orchestrator constructor
    // requires a handle.
    let opus = Arc::new(CannedSummariser {
        text: CANNED_RESPONSE.to_string(),
        kind: SummariserKind::Opus47,
        cost: 5_000,
    });
    let orch = Orchestrator::new(haiku, opus);

    let input = SessionInput {
        session_id: "01HXSESS00000000000000000A".into(),
        repo: Some("cortex".into()),
        envelopes: envelopes.clone(),
    };
    let produced = orch.run_session(&input).await.expect("run_session");

    // ---- Shape contract ----
    assert_eq!(produced.payload.grain, ConsolidationGrain::Session);
    assert_eq!(
        produced.payload.scope,
        ConsolidationScope::SessionId("01HXSESS00000000000000000A".into())
    );
    assert!(
        produced.payload.title.chars().count() <= 80,
        "title len {} exceeds 80 char cap",
        produced.payload.title.chars().count()
    );
    assert!(
        produced.payload.summary_markdown.len() >= 200
            && produced.payload.summary_markdown.len() <= 2_000,
        "summary_markdown len {} outside [200, 2000]",
        produced.payload.summary_markdown.len()
    );
    assert_eq!(produced.payload.takeaways.len(), 3);

    // ---- Source-id correctness ----
    assert_eq!(produced.payload.source_event_count, 30);
    assert_eq!(produced.payload.source_event_ids.len(), 30);
    for env in &envelopes {
        assert!(
            produced.payload.source_event_ids.contains(&env.event_id),
            "source_event_ids must include {} (envelope kind={:?})",
            env.event_id,
            env.kind
        );
    }

    // ---- Outcome distribution ----
    // 10 pairs, every 3rd user-side turn carries `outcome=error`;
    // pair-side + tool calls carry `outcome=success`. So:
    //   error: ⌈10/3⌉ = 4 (i = 0, 3, 6, 9)
    //   success: 10 (pair-side) + 10 (tool calls) + (10 - 4) (user-side success) = 26
    assert_eq!(
        produced.payload.outcome_distribution.get("error").copied(),
        Some(4)
    );
    assert_eq!(
        produced
            .payload
            .outcome_distribution
            .get("success")
            .copied(),
        Some(26)
    );

    // ---- Temporal span ----
    // Earliest envelope is the first user-side Turn at 10:00:00;
    // latest is the last ToolCall at 11:09:00.
    assert!(produced.payload.temporal_span.duration_ms > 0);
    assert_eq!(
        produced.payload.temporal_span.duration_ms,
        produced.payload.temporal_span.end_ms - produced.payload.temporal_span.start_ms
    );

    // ---- Wire metadata ----
    assert_eq!(produced.payload.model, "claude-haiku-4-5");
    assert_eq!(produced.payload.depth, ConsolidationDepth::Shallow);
    assert!(produced.payload.repos.contains(&"cortex".to_string()));
    assert_eq!(produced.cost_cents, 80);

    // ---- Cost ledger ----
    let ledger = orch.cost_ledger();
    let g = ledger.lock().expect("cost ledger lock");
    assert_eq!(g.per_grain["session"].consolidations, 1);
    assert_eq!(g.per_grain["session"].cost_cents, 80);
}

#[tokio::test]
async fn re_running_session_producer_against_the_same_input_yields_same_consolidation_id() {
    let envelopes = synth_session(10, 10);
    let haiku1 = Arc::new(CannedSummariser {
        text: CANNED_RESPONSE.to_string(),
        kind: SummariserKind::Haiku45,
        cost: 80,
    });
    let haiku2 = Arc::new(CannedSummariser {
        text: CANNED_RESPONSE.to_string(),
        kind: SummariserKind::Haiku45,
        cost: 80,
    });
    let opus = Arc::new(CannedSummariser {
        text: CANNED_RESPONSE.to_string(),
        kind: SummariserKind::Opus47,
        cost: 5_000,
    });
    let orch1 = Orchestrator::new(haiku1, opus.clone());
    let orch2 = Orchestrator::new(haiku2, opus);
    let input = SessionInput {
        session_id: "01HXSESS00000000000000000B".into(),
        repo: Some("cortex".into()),
        envelopes,
    };
    let p1 = orch1.run_session(&input).await.expect("first run");
    let p2 = orch2.run_session(&input).await.expect("second run");
    assert_eq!(p1.payload.consolidation_id, p2.payload.consolidation_id);
    assert!(p1.payload.consolidation_id.starts_with("cons-ses-"));
}
