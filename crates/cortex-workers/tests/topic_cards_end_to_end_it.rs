//! Phase11r §2.9 — end-to-end IT for the topic-cards orchestrator.
//!
//! Runs three sequential rewrites against a `CannedSummariser` so the
//! IT stays hermetic (no Anthropic API call). Asserts the contract
//! the producer + orchestrator pin: deterministic `topic_card_id`
//! across rewrites, monotonic `revision` (1 → 2 → 3), evidence
//! accumulation without duplication, contradictions surfaced when a
//! Decision supersession lands, cost ledger records under the
//! `topic_card` grain bucket.
//!
//! No `CORTEX_*_IT` env gate because everything runs in-process.

use std::sync::Arc;

use cortex_core::events::{
    derive_topic_card_id, ContradictionKind, EvidenceKind, EvidenceRef, TopicCardPayload,
};
use cortex_workers::consolidator::summariser::{
    Summariser, SummariserError, SummariserKind, SummariserRequest, SummariserResult,
};
use cortex_workers::topic_cards::orchestrator::{Orchestrator, GRAIN_LABEL};
use cortex_workers::topic_cards::producer::ProduceInput;

/// Tiny canned summariser — every invocation returns a fixed JSON
/// payload the producer parses back into a `RewriteOutput`. Lets the
/// IT exercise the full pipeline without an Anthropic round-trip.
struct CannedSummariser {
    kind: SummariserKind,
    body: String,
}

#[async_trait::async_trait]
impl Summariser for CannedSummariser {
    fn kind(&self) -> SummariserKind {
        self.kind
    }
    async fn summarise(
        &self,
        _req: SummariserRequest,
    ) -> Result<SummariserResult, SummariserError> {
        Ok(SummariserResult {
            text: self.body.clone(),
            cost_cents: 80,
            kind: self.kind,
            input_tokens: 1_000,
            output_tokens: 250,
        })
    }
}

fn canned_response(synthesis_filler: &str, with_contradiction: bool) -> String {
    let contradictions = if with_contradiction {
        serde_json::json!([
            {
                "kind": "decision_supersession",
                "evidence_a": "DEC-0042",
                "evidence_b": "DEC-0050"
            }
        ])
    } else {
        serde_json::json!([])
    };
    serde_json::to_string(&serde_json::json!({
        "synthesis_markdown": format!("# Auth rewrite\n\n{}\n\n{}", synthesis_filler, "x".repeat(300)),
        "contradictions": contradictions,
        "open_questions": ["does this preserve token rotation?"],
        "confidence": 0.82
    }))
    .expect("serialize canned response")
}

fn evidence_decision(id: &str) -> EvidenceRef {
    EvidenceRef {
        kind: EvidenceKind::Decision,
        id: id.to_string(),
        weight: None,
        cited_at_rev: 1,
    }
}

fn build_input(existing: Option<TopicCardPayload>, extra_evidence_ids: &[&str]) -> ProduceInput {
    // The producer trusts the caller to dedupe evidence by id — its
    // contract is "all_evidence is the canonical citation set" — so the
    // upstream orchestrator (or the smoke harness here) coalesces by id
    // before handing the input over.
    let mut all_evidence = existing
        .as_ref()
        .map(|c| c.evidence.clone())
        .unwrap_or_default();
    for id in extra_evidence_ids {
        if !all_evidence.iter().any(|e| e.id == *id) {
            all_evidence.push(evidence_decision(id));
        }
    }
    ProduceInput {
        topic_slug: "auth-rewrite".into(),
        repo_scope: "cortex".into(),
        existing_card: existing,
        all_evidence,
        new_evidence_text: extra_evidence_ids
            .iter()
            .map(|id| format!("- decision {id}"))
            .collect::<Vec<_>>()
            .join("\n"),
        superseded_evidence_text: String::new(),
        force_deep: false,
    }
}

#[tokio::test]
async fn three_sequential_rewrites_preserve_id_and_advance_revision() {
    let canned_first = canned_response("first revision", false);
    let canned_second = canned_response("second revision", false);
    let canned_third = canned_response("third revision with contradictions", true);

    // Round 1 — no existing card. Haiku.
    let haiku_1: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Haiku45,
        body: canned_first.clone(),
    });
    let opus_1: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Opus47,
        body: canned_first,
    });
    let orch_1 = Orchestrator::new(haiku_1, opus_1);
    let input_1 = build_input(None, &["DEC-0042"]);
    let produced_1 = orch_1.run(input_1).await.expect("rewrite 1 succeeds");
    assert_eq!(produced_1.payload.revision, 1);
    let expected_id = derive_topic_card_id("auth-rewrite", "cortex");
    assert_eq!(produced_1.payload.topic_card_id, expected_id);
    assert_eq!(produced_1.payload.evidence.len(), 1);
    assert_eq!(produced_1.payload.synthesis_model, "claude-haiku-4-5");
    assert_eq!(produced_1.cost_cents, 80);

    let ledger_1 = orch_1.cost_ledger();
    let ledger_lock = ledger_1.lock().unwrap();
    let bucket = ledger_lock
        .per_grain
        .get(GRAIN_LABEL)
        .expect("topic_card grain bucket exists");
    assert_eq!(bucket.cost_cents, 80);
    assert_eq!(bucket.consolidations, 1);
    drop(ledger_lock);

    // Round 2 — extend evidence. Same id, revision 2.
    let haiku_2: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Haiku45,
        body: canned_second.clone(),
    });
    let opus_2: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Opus47,
        body: canned_second,
    });
    let orch_2 = Orchestrator::new(haiku_2, opus_2);
    let input_2 = build_input(Some(produced_1.payload.clone()), &["DEC-0050"]);
    let produced_2 = orch_2.run(input_2).await.expect("rewrite 2 succeeds");
    assert_eq!(produced_2.payload.revision, 2);
    assert_eq!(produced_2.payload.topic_card_id, expected_id);
    // Evidence accumulated: prior 1 + new 1 = 2 (no duplicates because
    // DEC-0042 and DEC-0050 are distinct ids).
    assert_eq!(produced_2.payload.evidence.len(), 2);
    assert!(produced_2
        .payload
        .evidence
        .iter()
        .any(|e| e.id == "DEC-0042"));
    assert!(produced_2
        .payload
        .evidence
        .iter()
        .any(|e| e.id == "DEC-0050"));
    assert!(produced_2.payload.contradictions.is_empty());

    // Round 3 — re-cite DEC-0042 (must NOT duplicate). Canned response
    // surfaces a Decision supersession contradiction.
    let haiku_3: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Haiku45,
        body: canned_third.clone(),
    });
    let opus_3: Arc<dyn Summariser> = Arc::new(CannedSummariser {
        kind: SummariserKind::Opus47,
        body: canned_third,
    });
    let orch_3 = Orchestrator::new(haiku_3, opus_3);
    let input_3 = build_input(Some(produced_2.payload.clone()), &["DEC-0042"]);
    let produced_3 = orch_3.run(input_3).await.expect("rewrite 3 succeeds");
    assert_eq!(produced_3.payload.revision, 3);
    assert_eq!(produced_3.payload.topic_card_id, expected_id);
    // Evidence still 2 — the producer dedupes by id when extending.
    assert_eq!(produced_3.payload.evidence.len(), 2);

    // Contradictions surfaced from the canned model output.
    assert_eq!(produced_3.payload.contradictions.len(), 1);
    let contradiction = &produced_3.payload.contradictions[0];
    assert!(matches!(
        contradiction.kind,
        ContradictionKind::DecisionSupersession
    ));
    assert_eq!(contradiction.surfaced_at_rev, 3);
}
