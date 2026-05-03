//! Runtime JSON-Schema validator for events.
//!
//! Schemas under `schemas/` are embedded at compile time so validation works
//! without any on-disk lookup in downstream crates.

use crate::events::Kind;
use jsonschema::Validator as JsonValidator;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::collections::HashMap;

/// Schema validation failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ValidationError {
    /// A JSON pointer-style error path from the underlying validator.
    #[error("{path}: {message}")]
    Schema {
        /// Instance pointer into the document.
        path: String,
        /// Validator message.
        message: String,
    },
    /// Envelope's declared `kind` is unknown.
    #[error("unknown kind: {0}")]
    UnknownKind(String),
    /// Envelope is missing `kind`.
    #[error("envelope missing `kind`")]
    MissingKind,
    /// Envelope is missing `payload`.
    #[error("envelope missing `payload`")]
    MissingPayload,
}

const ENVELOPE_SCHEMA: &str = include_str!("../schemas/envelope.schema.json");
const CONTEXT_SCHEMA: &str = include_str!("../schemas/context.schema.json");
const TURN_SCHEMA: &str = include_str!("../schemas/kinds/turn.schema.json");
const TOOL_CALL_SCHEMA: &str = include_str!("../schemas/kinds/tool_call.schema.json");
const AGENT_CALL_SCHEMA: &str = include_str!("../schemas/kinds/agent_call.schema.json");
const MEMORY_SCHEMA: &str = include_str!("../schemas/kinds/memory.schema.json");
const DECISION_SCHEMA: &str = include_str!("../schemas/kinds/decision.schema.json");
const ANALYSIS_SCHEMA: &str = include_str!("../schemas/kinds/analysis.schema.json");
const LAW_VIOLATION_SCHEMA: &str = include_str!("../schemas/kinds/law_violation.schema.json");
const ARTIFACT_SCHEMA: &str = include_str!("../schemas/kinds/artifact.schema.json");
const CONSOLIDATION_SCHEMA: &str = include_str!("../schemas/kinds/consolidation.schema.json");
const TOPIC_CARD_SCHEMA: &str = include_str!("../schemas/kinds/topic_card.schema.json");

static VALIDATOR: Lazy<Validator> =
    Lazy::new(|| Validator::new().expect("embedded schemas must always compile"));

/// Compiled schema set used to validate envelopes + per-kind payloads.
pub struct Validator {
    envelope: JsonValidator,
    /// Map kind-stem → compiled schema.
    kinds: HashMap<&'static str, JsonValidator>,
}

impl Validator {
    /// Compile every embedded schema. Expensive — call once and share.
    pub fn new() -> Result<Self, String> {
        // The envelope schema `$ref`s the context schema by relative URI. We
        // pre-register it so the validator resolves in-process.
        let context_uri = "context.schema.json";
        let context_value: Value = serde_json::from_str(CONTEXT_SCHEMA)
            .map_err(|e| format!("context schema parse: {e}"))?;

        let envelope_value: Value = serde_json::from_str(ENVELOPE_SCHEMA)
            .map_err(|e| format!("envelope schema parse: {e}"))?;
        let registry = jsonschema::Registry::new()
            .add(
                context_uri,
                jsonschema::Resource::from_contents(context_value),
            )
            .map_err(|e| format!("context resource: {e}"))?
            .prepare()
            .map_err(|e| format!("context registry: {e}"))?;
        let envelope = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .with_registry(&registry)
            .build(&envelope_value)
            .map_err(|e| format!("envelope schema compile: {e}"))?;

        let kinds: HashMap<&'static str, JsonValidator> = [
            ("turn", compile(TURN_SCHEMA)?),
            ("tool_call", compile(TOOL_CALL_SCHEMA)?),
            ("agent_call", compile(AGENT_CALL_SCHEMA)?),
            ("memory", compile(MEMORY_SCHEMA)?),
            ("decision", compile(DECISION_SCHEMA)?),
            ("analysis", compile(ANALYSIS_SCHEMA)?),
            ("law_violation", compile(LAW_VIOLATION_SCHEMA)?),
            ("artifact", compile(ARTIFACT_SCHEMA)?),
            ("consolidation", compile(CONSOLIDATION_SCHEMA)?),
            ("topic_card", compile(TOPIC_CARD_SCHEMA)?),
        ]
        .into_iter()
        .collect();

        Ok(Validator { envelope, kinds })
    }

    /// Return the shared, lazily-compiled validator.
    pub fn shared() -> &'static Validator {
        &VALIDATOR
    }

    /// Validate just the envelope shape (no payload inspection).
    pub fn validate_envelope(&self, value: &Value) -> Result<(), Vec<ValidationError>> {
        collect(&self.envelope, value)
    }

    /// Validate an envelope plus the matching per-kind payload.
    pub fn validate_event(&self, value: &Value) -> Result<(), Vec<ValidationError>> {
        let mut errors = collect(&self.envelope, value).err().unwrap_or_default();

        let kind_val = value.get("kind").and_then(|k| k.as_str());
        let payload = value.get("payload");

        match (kind_val, payload) {
            (Some(k), Some(p)) => {
                if let Some(schema) = self.kinds.get(k) {
                    if let Err(mut payload_errors) = collect(schema, p) {
                        // Re-root the path so callers can see it lives under `payload.*`.
                        for e in &mut payload_errors {
                            if let ValidationError::Schema { path, .. } = e {
                                *path = if path == "/" || path.is_empty() {
                                    "/payload".into()
                                } else {
                                    format!("/payload{}", path)
                                };
                            }
                        }
                        errors.append(&mut payload_errors);
                    }
                } else {
                    errors.push(ValidationError::UnknownKind(k.to_string()));
                }
            }
            (None, _) => errors.push(ValidationError::MissingKind),
            (_, None) => errors.push(ValidationError::MissingPayload),
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Validate an event against the shared [`Validator`].
pub fn validate_event(value: &Value) -> Result<(), Vec<ValidationError>> {
    Validator::shared().validate_event(value)
}

/// Validate an envelope's shape against the shared [`Validator`] (no payload check).
pub fn validate_envelope(value: &Value) -> Result<(), Vec<ValidationError>> {
    Validator::shared().validate_envelope(value)
}

/// Compile a JSON Schema draft 2020-12 document.
fn compile(src: &str) -> Result<JsonValidator, String> {
    let value: Value = serde_json::from_str(src).map_err(|e| format!("schema parse: {e}"))?;
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&value)
        .map_err(|e| format!("schema compile: {e}"))
}

fn collect(schema: &JsonValidator, value: &Value) -> Result<(), Vec<ValidationError>> {
    let mut out = Vec::new();
    for err in schema.iter_errors(value) {
        out.push(ValidationError::Schema {
            path: err.instance_path().to_string(),
            message: err.to_string(),
        });
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

/// Shorthand: return the schema-file stem for a [`Kind`].
pub fn schema_stem_for(kind: Kind) -> &'static str {
    kind.schema_stem()
}

/// Phase11j §1.5 — cross-field rule the JSON Schema cannot
/// express cleanly: the `scope` variant must match the `grain`
/// discriminator. Returns `Ok(())` on a match,
/// `Err(ValidationError::Schema { … })` when the pair is
/// inconsistent. Callers should run this *after* the per-kind
/// JSON Schema check; it operates on a typed
/// [`crate::events::ConsolidationPayload`] so the field-level
/// invariants the schema already enforced (lengths, integer
/// ranges) are guaranteed by the time we get here.
pub fn validate_consolidation_payload(
    payload: &crate::events::ConsolidationPayload,
) -> Result<(), ValidationError> {
    use crate::events::{ConsolidationGrain, ConsolidationScope};
    let pair_ok = matches!(
        (payload.grain, &payload.scope),
        (
            ConsolidationGrain::Session,
            ConsolidationScope::SessionId(_)
        ) | (ConsolidationGrain::Topic, ConsolidationScope::Topic(_))
            | (
                ConsolidationGrain::DecisionTrace,
                ConsolidationScope::DecisionId(_),
            )
    );
    if !pair_ok {
        return Err(ValidationError::Schema {
            path: "/payload/scope".into(),
            message: format!("scope variant does not match grain {:?}", payload.grain),
        });
    }
    if payload.source_event_count < payload.source_event_ids.len() as u32 {
        return Err(ValidationError::Schema {
            path: "/payload/source_event_count".into(),
            message: format!(
                "source_event_count ({}) below source_event_ids.len() ({})",
                payload.source_event_count,
                payload.source_event_ids.len()
            ),
        });
    }
    if payload.temporal_span.end_ms < payload.temporal_span.start_ms {
        return Err(ValidationError::Schema {
            path: "/payload/temporal_span".into(),
            message: format!(
                "end_ms ({}) before start_ms ({})",
                payload.temporal_span.end_ms, payload.temporal_span.start_ms
            ),
        });
    }
    let materialised = payload.temporal_span.end_ms - payload.temporal_span.start_ms;
    if payload.temporal_span.duration_ms != materialised {
        return Err(ValidationError::Schema {
            path: "/payload/temporal_span/duration_ms".into(),
            message: format!(
                "duration_ms ({}) does not equal end_ms - start_ms ({})",
                payload.temporal_span.duration_ms, materialised
            ),
        });
    }
    Ok(())
}

/// Phase11r §1.5 — validate the cross-field invariants the JSON Schema
/// cannot express on a [`crate::events::TopicCardPayload`]:
///
/// 1. `evidence` is non-empty.
/// 2. Every `contradictions[*].evidence_a` / `evidence_b` references
///    an `EvidenceRef.id` present in the same payload's `evidence`
///    vector. Orphan references would let the renderer point at a
///    citation the operator cannot fetch.
///
/// Returns `Ok(())` on a match, `Err(ValidationError::Schema { … })`
/// otherwise. Callers should run this *after* the per-kind JSON
/// Schema check.
pub fn validate_topic_card_payload(
    payload: &crate::events::TopicCardPayload,
) -> Result<(), ValidationError> {
    if payload.evidence.is_empty() {
        return Err(ValidationError::Schema {
            path: "/payload/evidence".into(),
            message: "evidence_required: at least one EvidenceRef must be cited".into(),
        });
    }
    let known: std::collections::HashSet<&str> = payload
        .evidence
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    for (idx, c) in payload.contradictions.iter().enumerate() {
        if !known.contains(c.evidence_a.as_str()) {
            return Err(ValidationError::Schema {
                path: format!("/payload/contradictions/{idx}/evidence_a"),
                message: format!(
                    "contradiction_references_unknown_evidence: {}",
                    c.evidence_a
                ),
            });
        }
        if !known.contains(c.evidence_b.as_str()) {
            return Err(ValidationError::Schema {
                path: format!("/payload/contradictions/{idx}/evidence_b"),
                message: format!(
                    "contradiction_references_unknown_evidence: {}",
                    c.evidence_b
                ),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod topic_card_validate_tests {
    use super::*;
    use crate::events::{
        Contradiction, ContradictionKind, ContradictionStatus, EvidenceKind, EvidenceRef,
        TopicCardPayload,
    };

    fn ok_payload() -> TopicCardPayload {
        TopicCardPayload {
            topic_card_id: crate::events::derive_topic_card_id("auth-rewrite", "cortex"),
            topic_slug: "auth-rewrite".into(),
            repos: vec!["cortex".into()],
            revision: 1,
            synthesis_markdown: "x".repeat(400),
            evidence: vec![EvidenceRef {
                kind: EvidenceKind::Decision,
                id: "DEC-0042".into(),
                weight: None,
                cited_at_rev: 1,
            }],
            contradictions: vec![],
            open_questions: vec![],
            related_topic_ids: vec![],
            confidence: 0.85,
            last_rev_at: "2026-05-03T05:00:00Z".into(),
            events_since_last_rev: 0,
            synthesis_model: "claude-haiku-4-5".into(),
            synthesis_cost_cents: 80,
        }
    }

    #[test]
    fn accepts_minimal_payload() {
        validate_topic_card_payload(&ok_payload()).expect("baseline ok");
    }

    #[test]
    fn rejects_empty_evidence() {
        let mut p = ok_payload();
        p.evidence.clear();
        let err = validate_topic_card_payload(&p).expect_err("empty evidence");
        match err {
            ValidationError::Schema { path, message } => {
                assert_eq!(path, "/payload/evidence");
                assert!(message.contains("evidence_required"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_orphan_contradiction_evidence_a() {
        let mut p = ok_payload();
        p.contradictions.push(Contradiction {
            kind: ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-9999".into(), // not in evidence
            evidence_b: "DEC-0042".into(),
            surfaced_at_rev: 1,
            status: ContradictionStatus::Open,
        });
        let err = validate_topic_card_payload(&p).expect_err("orphan a");
        match err {
            ValidationError::Schema { path, message } => {
                assert_eq!(path, "/payload/contradictions/0/evidence_a");
                assert!(message.contains("contradiction_references_unknown_evidence"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_orphan_contradiction_evidence_b() {
        let mut p = ok_payload();
        p.contradictions.push(Contradiction {
            kind: ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-0042".into(),
            evidence_b: "DEC-7777".into(), // not in evidence
            surfaced_at_rev: 1,
            status: ContradictionStatus::Open,
        });
        let err = validate_topic_card_payload(&p).expect_err("orphan b");
        match err {
            ValidationError::Schema { path, .. } => {
                assert_eq!(path, "/payload/contradictions/0/evidence_b");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn accepts_contradiction_with_both_refs_in_evidence() {
        let mut p = ok_payload();
        p.evidence.push(EvidenceRef {
            kind: EvidenceKind::Decision,
            id: "DEC-0050".into(),
            weight: None,
            cited_at_rev: 1,
        });
        p.contradictions.push(Contradiction {
            kind: ContradictionKind::DecisionSupersession,
            evidence_a: "DEC-0042".into(),
            evidence_b: "DEC-0050".into(),
            surfaced_at_rev: 1,
            status: ContradictionStatus::Open,
        });
        validate_topic_card_payload(&p).expect("both refs in evidence");
    }

    #[test]
    fn schema_validator_rejects_synthesis_below_floor() {
        let envelope = serde_json::json!({
            "event_id": "01HX0000000000000000000001",
            "schema_version": "1",
            "occurred_at": "2026-05-03T05:00:00Z",
            "session_id": "01HXY00000000000000000000Z",
            "stream": "live",
            "tool": "claude-code",
            "kind": "topic_card",
            "context": {
                "platform": "linux"
            },
            "payload": {
                "topic_card_id": "topic-0123456789abcdef01234567",
                "topic_slug": "auth-rewrite",
                "repos": ["cortex"],
                "revision": 1,
                "synthesis_markdown": "tooshort",
                "evidence": [
                    {"kind": "decision", "id": "DEC-0042", "cited_at_rev": 1}
                ],
                "confidence": 0.5,
                "last_rev_at": "2026-05-03T05:00:00Z",
                "events_since_last_rev": 0,
                "synthesis_model": "claude-haiku-4-5",
                "synthesis_cost_cents": 80
            },
            "content_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "redactions": []
        });
        let v = Validator::shared();
        let err = v.validate_event(&envelope).expect_err("below floor");
        let mentions_synthesis = err.iter().any(|e| match e {
            ValidationError::Schema { path, .. } => path.contains("synthesis_markdown"),
            _ => false,
        });
        assert!(mentions_synthesis, "errors did not flag synthesis_markdown: {err:?}");
    }

    #[test]
    fn schema_validator_accepts_minimal_envelope() {
        let envelope = serde_json::json!({
            "event_id": "01HX0000000000000000000001",
            "schema_version": "1",
            "occurred_at": "2026-05-03T05:00:00Z",
            "session_id": "01HXY00000000000000000000Z",
            "stream": "live",
            "tool": "claude-code",
            "kind": "topic_card",
            "context": {
                "platform": "linux"
            },
            "payload": {
                "topic_card_id": "topic-0123456789abcdef01234567",
                "topic_slug": "auth-rewrite",
                "repos": ["cortex"],
                "revision": 1,
                "synthesis_markdown": "x".repeat(400),
                "evidence": [
                    {"kind": "decision", "id": "DEC-0042", "cited_at_rev": 1}
                ],
                "confidence": 0.5,
                "last_rev_at": "2026-05-03T05:00:00Z",
                "events_since_last_rev": 0,
                "synthesis_model": "claude-haiku-4-5",
                "synthesis_cost_cents": 80
            },
            "content_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "redactions": []
        });
        let v = Validator::shared();
        v.validate_event(&envelope).expect("minimal envelope ok");
    }

    #[test]
    fn schema_validator_rejects_invalid_slug() {
        let envelope = serde_json::json!({
            "event_id": "01HX0000000000000000000001",
            "schema_version": "1",
            "occurred_at": "2026-05-03T05:00:00Z",
            "session_id": "01HXY00000000000000000000Z",
            "stream": "live",
            "tool": "claude-code",
            "kind": "topic_card",
            "context": {
                "platform": "linux"
            },
            "payload": {
                "topic_card_id": "topic-0123456789abcdef01234567",
                "topic_slug": "Auth_Rewrite",
                "repos": ["cortex"],
                "revision": 1,
                "synthesis_markdown": "x".repeat(400),
                "evidence": [
                    {"kind": "decision", "id": "DEC-0042", "cited_at_rev": 1}
                ],
                "confidence": 0.5,
                "last_rev_at": "2026-05-03T05:00:00Z",
                "events_since_last_rev": 0,
                "synthesis_model": "claude-haiku-4-5",
                "synthesis_cost_cents": 80
            },
            "content_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "redactions": []
        });
        let v = Validator::shared();
        let err = v.validate_event(&envelope).expect_err("uppercase slug");
        let mentions_slug = err.iter().any(|e| match e {
            ValidationError::Schema { path, .. } => path.contains("topic_slug"),
            _ => false,
        });
        assert!(mentions_slug, "errors did not flag topic_slug: {err:?}");
    }
}

#[cfg(test)]
mod consolidation_validate_tests {
    use super::*;
    use crate::events::{
        ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, TimeSpan,
    };
    use std::collections::BTreeMap;

    fn ok_payload() -> ConsolidationPayload {
        ConsolidationPayload {
            consolidation_id: "01CON".into(),
            grain: ConsolidationGrain::Topic,
            scope: ConsolidationScope::Topic("hnsw".into()),
            title: "topic: hnsw recall".into(),
            summary_markdown: "x".repeat(400),
            takeaways: vec![],
            source_event_ids: vec!["01EVT".into()],
            source_event_count: 1,
            model: "claude-haiku-4-5".into(),
            depth: ConsolidationDepth::Shallow,
            outcome_distribution: BTreeMap::new(),
            temporal_span: TimeSpan {
                start_ms: 100,
                end_ms: 200,
                duration_ms: 100,
            },
            repos: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn rust_validator_accepts_matching_grain_and_scope() {
        let p = ok_payload();
        validate_consolidation_payload(&p).expect("matched grain+scope");
    }

    #[test]
    fn rust_validator_rejects_mismatched_grain_and_scope() {
        let mut p = ok_payload();
        p.grain = ConsolidationGrain::Session;
        let err = validate_consolidation_payload(&p).expect_err("mismatch");
        match err {
            ValidationError::Schema { path, message } => {
                assert_eq!(path, "/payload/scope");
                assert!(message.contains("grain"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn rust_validator_rejects_count_below_inline_ids_len() {
        let mut p = ok_payload();
        p.source_event_ids = vec!["a".into(), "b".into(), "c".into()];
        p.source_event_count = 2;
        let err = validate_consolidation_payload(&p).expect_err("count<len");
        match err {
            ValidationError::Schema { path, .. } => {
                assert_eq!(path, "/payload/source_event_count");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn rust_validator_rejects_inverted_temporal_span() {
        let mut p = ok_payload();
        p.temporal_span = TimeSpan {
            start_ms: 200,
            end_ms: 100,
            duration_ms: -100,
        };
        let err = validate_consolidation_payload(&p).expect_err("inverted span");
        match err {
            ValidationError::Schema { path, .. } => {
                assert_eq!(path, "/payload/temporal_span");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn rust_validator_rejects_inconsistent_duration_ms() {
        let mut p = ok_payload();
        p.temporal_span.duration_ms = 999;
        let err = validate_consolidation_payload(&p).expect_err("duration drift");
        match err {
            ValidationError::Schema { path, .. } => {
                assert_eq!(path, "/payload/temporal_span/duration_ms");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn json_schema_accepts_minimal_consolidation_payload() {
        // The schema-side path: build a synthetic envelope and run
        // it through `validate_event`. The fixture below pins the
        // shape so a schema regression breaks this test.
        let envelope = serde_json::json!({
            "event_id": "01HXEVT0000000000000000000",
            "schema_version": "1",
            "occurred_at": "2026-04-20T17:47:59.616Z",
            "stream": "live",
            "tool": "cortex-cli",
            "kind": "consolidation",
            "session_id": "01HXSESS00000000000000000A",
            "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "context": { "platform": "linux" },
            "payload": {
                "consolidation_id": "01CON",
                "grain": "session",
                "scope": { "kind": "session_id", "value": "sess-A" },
                "title": "ok",
                "summary_markdown": "x".repeat(200),
                "source_event_count": 0,
                "model": "claude-haiku-4-5",
                "depth": "shallow",
                "temporal_span": { "start_ms": 0, "end_ms": 0, "duration_ms": 0 }
            }
        });
        validate_event(&envelope).expect("minimal payload validates");
    }

    #[test]
    fn json_schema_rejects_summary_below_two_hundred_bytes() {
        let envelope = serde_json::json!({
            "event_id": "01HXEVT0000000000000000000",
            "schema_version": "1",
            "occurred_at": "2026-04-20T17:47:59.616Z",
            "stream": "live",
            "tool": "cortex-cli",
            "kind": "consolidation",
            "session_id": "01HXSESS00000000000000000A",
            "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "context": { "platform": "linux" },
            "payload": {
                "consolidation_id": "01CON",
                "grain": "session",
                "scope": { "kind": "session_id", "value": "sess-A" },
                "title": "ok",
                "summary_markdown": "too short",
                "source_event_count": 0,
                "model": "m",
                "depth": "shallow",
                "temporal_span": { "start_ms": 0, "end_ms": 0, "duration_ms": 0 }
            }
        });
        let err = validate_event(&envelope).expect_err("summary too short");
        assert!(
            err.iter().any(|e| matches!(e, ValidationError::Schema { path, .. } if path.contains("summary_markdown"))),
            "expected summary_markdown error, got {err:?}"
        );
    }

    #[test]
    fn json_schema_rejects_unknown_grain() {
        let envelope = serde_json::json!({
            "event_id": "01HXEVT0000000000000000000",
            "schema_version": "1",
            "occurred_at": "2026-04-20T17:47:59.616Z",
            "stream": "live",
            "tool": "cortex-cli",
            "kind": "consolidation",
            "session_id": "01HXSESS00000000000000000A",
            "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "context": { "platform": "linux" },
            "payload": {
                "consolidation_id": "01CON",
                "grain": "future-grain",
                "scope": { "kind": "session_id", "value": "sess-A" },
                "title": "ok",
                "summary_markdown": "x".repeat(200),
                "source_event_count": 0,
                "model": "m",
                "depth": "shallow",
                "temporal_span": { "start_ms": 0, "end_ms": 0, "duration_ms": 0 }
            }
        });
        let err = validate_event(&envelope).expect_err("unknown grain");
        assert!(
            err.iter().any(
                |e| matches!(e, ValidationError::Schema { path, .. } if path.contains("grain"))
            ),
            "expected grain error, got {err:?}"
        );
    }
}
