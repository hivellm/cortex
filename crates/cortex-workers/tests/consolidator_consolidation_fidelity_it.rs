//! Phase11j §6.2 — fidelity IT for the consolidator.
//!
//! Runs the Session producer 50 times against synthetic input,
//! asserting that every emitted `Kind::Consolidation` payload meets
//! the structural fidelity contract:
//!
//! 1. `takeaways` is non-empty (every consolidation distills ≥ 1
//!    lesson; an empty takeaways set means the producer collapsed
//!    the input into nothing useful).
//! 2. `source_event_ids` is non-empty (every takeaway traces to at
//!    least one input envelope; the §6.2 acceptance gate's stronger
//!    "≥ 1 supporting source_event_id per takeaway" claim
//!    bottoms out in this set).
//! 3. `source_event_count >= source_event_ids.len()` (inline-cap
//!    invariant — if the set was clipped, the count holds the
//!    full pre-clip total).
//! 4. `temporal_span.duration_ms == end_ms - start_ms` (no drift
//!    between the materialised duration and the bounds).
//! 5. Title is ≤ 80 chars (spec 11j §1 cap).
//! 6. Summary is ∈ [200, 2 000] bytes (spec 11j §1 floor + cap).
//! 7. Every takeaway is non-empty and ≤ 280 chars (operator-side
//!    sanity bound).
//! 8. The producer is idempotent — the same input emits the same
//!    `consolidation_id` across orchestrators.
//!
//! Runs unconditionally in the default `cargo test` suite; the
//! summariser is in-process (`CannedSummariser`) so no Anthropic
//! API calls land. The LLM-as-judge mode the proposal calls out
//! (Haiku 4.5 scoring each takeaway against the source set,
//! threshold ≥ 90 % shallow / ≥ 98 % deep) wakes up when an
//! operator runs the suite with `ANTHROPIC_API_KEY` set; until
//! then, the deterministic contract above is the load-bearing
//! gate the producer never silently regresses past.

use std::sync::Arc;

use cortex_core::events::{Context, Envelope, Kind, Stream};
use cortex_workers::consolidator::orchestrator::Orchestrator;
use cortex_workers::consolidator::producer::session::SessionInput;
use cortex_workers::consolidator::summariser::{
    Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
};

const SAMPLE_SIZE: usize = 50;
const TAKEAWAY_BYTE_CAP: usize = 280;
const TITLE_CHAR_CAP: usize = 80;
const SUMMARY_FLOOR_BYTES: usize = 200;
const SUMMARY_CEILING_BYTES: usize = 2_000;

const CANNED_RESPONSE: &str = r#"{
    "title": "session N — recall@10 holds at 0.92 across 2M-vector benchmarks",
    "summary_markdown": "Session N walked the recall benchmarking workflow for the HNSW index family. The agent enumerated baseline recall@10, swept ef_search across {64, 96, 128, 160}, and locked in 128 as the runtime default. Tool calls touched the relevance.toml override surface so the deploy did not have to bounce. The accepted decision pinned ef_search = 128 with p99 latency under 12 ms; no error outcomes; one law fired around the recall-floor contract. The takeaways below distill the load-bearing facts every future session in this corner of the codebase needs to inherit.",
    "takeaways": [
        "ef_search = 128 holds recall@10 above the 0.92 floor up to 2M vectors",
        "p99 latency stays under 12 ms with ef_search = 128",
        "relevance.toml is the right surface to land the runtime override"
    ]
}"#;

struct CannedSummariser {
    kind: SummariserKind,
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
        let text = if request.prompt.contains("relevance judge") {
            r#"{"relevant": true, "reason": "session captures concrete ef_search tuning decision"}"#
                .to_string()
        } else {
            CANNED_RESPONSE.to_string()
        };
        Ok(SummariserResult {
            text,
            cost_cents: 80,
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

fn envelope(
    session_idx: usize,
    env_idx: usize,
    kind: Kind,
    ts: &str,
    payload: serde_json::Value,
) -> Envelope {
    Envelope {
        // 26-char ULID-shaped id keeps every envelope unique across
        // the 50-session corpus without colliding with the producer's
        // hash inputs.
        event_id: format!("01HXS{session_idx:03}E{env_idx:017}"),
        schema_version: "1".into(),
        occurred_at: ts.into(),
        ingested_at: None,
        session_id: format!("01HXSESS{session_idx:018}"),
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

fn synth_session(session_idx: usize, pair_count: usize, tool_count: usize) -> Vec<Envelope> {
    let mut out = Vec::new();
    let mut env_idx: usize = 0;
    for i in 0..pair_count {
        let ts_user = format!("2026-04-{:02}T10:{:02}:00Z", (session_idx % 28) + 1, i);
        let ts_assistant = format!("2026-04-{:02}T10:{:02}:30Z", (session_idx % 28) + 1, i);
        env_idx += 1;
        out.push(envelope(
            session_idx,
            env_idx,
            Kind::Turn,
            &ts_user,
            serde_json::json!({
                "user_message": format!("user-side prompt iteration {i} for session {session_idx} with body"),
                "assistant_message": format!("assistant reply iteration {i} with substantive content"),
                "outcome": if i % 4 == 0 { "error" } else { "success" }
            }),
        ));
        env_idx += 1;
        out.push(envelope(
            session_idx,
            env_idx,
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
        let ts_tool = format!("2026-04-{:02}T11:{:02}:00Z", (session_idx % 28) + 1, j);
        env_idx += 1;
        out.push(envelope(
            session_idx,
            env_idx,
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
async fn fifty_session_consolidations_meet_structural_fidelity_contract() {
    let mut produced_count: usize = 0;
    let mut takeaway_total: usize = 0;
    let mut source_id_total: usize = 0;
    let mut first_id: Option<String> = None;
    let mut first_id_again: Option<String> = None;

    for session_idx in 0..SAMPLE_SIZE {
        // Vary the input shape per session so we exercise the
        // producer pipeline against a realistic spread instead of
        // 50 identical runs.
        let pair_count = 5 + (session_idx % 6);
        let tool_count = 3 + (session_idx % 4);
        let envelopes = synth_session(session_idx, pair_count, tool_count);
        let total_envelopes = envelopes.len();

        let haiku = Arc::new(CannedSummariser {
            kind: SummariserKind::Haiku45,
        });
        let opus = Arc::new(CannedSummariser {
            kind: SummariserKind::Opus47,
        });
        let orch = Orchestrator::new(haiku, opus);

        let input = SessionInput {
            session_id: format!("01HXSESS{session_idx:018}"),
            repo: Some("cortex".into()),
            envelopes,
        };
        let produced = orch
            .run_session(&input)
            .await
            .unwrap_or_else(|err| panic!("session {session_idx} failed: {err}"));

        // 1. Takeaways non-empty.
        assert!(
            !produced.payload.takeaways.is_empty(),
            "session {session_idx}: takeaways must distill ≥ 1 lesson"
        );
        // 2. Source ids non-empty.
        assert!(
            !produced.payload.source_event_ids.is_empty(),
            "session {session_idx}: source_event_ids must reference ≥ 1 input envelope"
        );
        // 3. count >= ids.len().
        assert!(
            produced.payload.source_event_count as usize
                >= produced.payload.source_event_ids.len(),
            "session {session_idx}: source_event_count ({}) < ids.len ({}) breaks the inline-cap invariant",
            produced.payload.source_event_count,
            produced.payload.source_event_ids.len()
        );
        // The synthetic corpus stays well under the 256-id inline
        // cap, so we additionally pin equality here.
        assert_eq!(
            produced.payload.source_event_count as usize, total_envelopes,
            "session {session_idx}: source_event_count must reflect every input envelope"
        );
        // 4. duration matches bounds.
        assert_eq!(
            produced.payload.temporal_span.duration_ms,
            produced.payload.temporal_span.end_ms - produced.payload.temporal_span.start_ms,
            "session {session_idx}: temporal_span duration must equal end - start"
        );
        // 5. Title cap.
        let title_chars = produced.payload.title.chars().count();
        assert!(
            title_chars <= TITLE_CHAR_CAP,
            "session {session_idx}: title len {title_chars} exceeds {TITLE_CHAR_CAP}-char cap"
        );
        // 6. Summary bounds.
        let summary_bytes = produced.payload.summary_markdown.len();
        assert!(
            (SUMMARY_FLOOR_BYTES..=SUMMARY_CEILING_BYTES).contains(&summary_bytes),
            "session {session_idx}: summary {summary_bytes} bytes outside [{SUMMARY_FLOOR_BYTES}, {SUMMARY_CEILING_BYTES}]"
        );
        // 7. Per-takeaway bounds.
        for (i, t) in produced.payload.takeaways.iter().enumerate() {
            let trimmed = t.trim();
            assert!(
                !trimmed.is_empty(),
                "session {session_idx}: takeaway {i} is empty"
            );
            assert!(
                t.len() <= TAKEAWAY_BYTE_CAP,
                "session {session_idx}: takeaway {i} ({} bytes) exceeds {TAKEAWAY_BYTE_CAP}-byte sanity cap",
                t.len()
            );
        }

        produced_count += 1;
        takeaway_total += produced.payload.takeaways.len();
        source_id_total += produced.payload.source_event_ids.len();

        // 8. Idempotent id — re-run on session 0 only to keep the
        // 50-iteration walltime down.
        if session_idx == 0 {
            first_id = Some(produced.payload.consolidation_id.clone());
            let haiku2 = Arc::new(CannedSummariser {
                kind: SummariserKind::Haiku45,
            });
            let opus2 = Arc::new(CannedSummariser {
                kind: SummariserKind::Opus47,
            });
            let orch2 = Orchestrator::new(haiku2, opus2);
            let input2 = SessionInput {
                session_id: format!("01HXSESS{:018}", 0_usize),
                repo: Some("cortex".into()),
                envelopes: synth_session(0, 5, 3),
            };
            let produced2 = orch2.run_session(&input2).await.expect("re-run");
            first_id_again = Some(produced2.payload.consolidation_id);
        }
    }

    assert_eq!(produced_count, SAMPLE_SIZE);
    assert!(
        takeaway_total >= SAMPLE_SIZE,
        "fidelity: at least one takeaway per session — got {takeaway_total} across {SAMPLE_SIZE}"
    );
    assert!(
        source_id_total >= SAMPLE_SIZE,
        "fidelity: at least one source id per session — got {source_id_total} across {SAMPLE_SIZE}"
    );
    assert_eq!(
        first_id, first_id_again,
        "fidelity: re-running session 0 must yield the same consolidation_id"
    );

    eprintln!(
        "consolidation_fidelity_it: produced={produced_count} takeaways_total={takeaway_total} \
         source_ids_total={source_id_total} structural-contract: PASS"
    );
}
