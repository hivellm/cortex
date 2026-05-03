//! Classifier types.

use async_trait::async_trait;
use cortex_core::events::Kind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Severity label attached to every classified event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Routine, low-signal activity.
    Info,
    /// Worth surfacing in the dashboard timeline.
    Notable,
    /// Must alert — security, broken contract, law violation, data loss.
    Critical,
}

impl Severity {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Notable => "notable",
            Severity::Critical => "critical",
        }
    }
}

/// PII risk label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PiiRisk {
    /// No personal data, no secrets.
    Low,
    /// Usernames, emails, internal paths, repo/branch names.
    Medium,
    /// Credentials, tokens, keys, financial data, customer PII.
    High,
}

impl PiiRisk {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            PiiRisk::Low => "low",
            PiiRisk::Medium => "medium",
            PiiRisk::High => "high",
        }
    }
}

/// Redaction suggestion from the classifier — a secret the static redactor missed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionSuggestion {
    /// Short classification tag.
    pub pattern_class: String,
    /// JSON pointer into the payload.
    pub json_pointer: String,
    /// Free-form explanation from the classifier.
    pub rationale: String,
}

/// One typed entity the classifier identified inside an event.
/// These become Nexus nodes in the graph mapper, keyed by
/// `(entity_type, identifier)` so the same `Decision DEC-0042`
/// referenced from multiple events collapses to a single graph
/// node — exactly the cross-event interconnection the user asked
/// for. The 2026-04-28 audit captured the gap: Cortex's Nexus
/// graph today carries only structural edges (HAS_TURN / TOUCHED)
/// and no semantic ones, so retrieval by topic / decision / law
/// can't traverse the entity layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// Canonical entity type. The classifier picks from a
    /// controlled list so the graph mapper can route to the right
    /// Nexus label without free-form name munging:
    /// `decision` | `law` | `analysis` | `artifact` | `repo` |
    /// `topic` | `concept` | `tool` | `person` | `session` | `turn`.
    pub entity_type: String,
    /// Unique-within-type identifier. For decisions this is the
    /// `DEC-NNNN` id; for files it's the repo-rooted path; for
    /// concepts / topics it's a slug. The mapper composes the
    /// natural key as `{entity_type}|{identifier}` so two events
    /// referencing the same decision land on the same node.
    pub identifier: String,
    /// Optional human-readable label. The mapper falls back to
    /// `identifier` when this is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One semantic relation the classifier identified between this
/// event and a previously-listed entity (or another event /
/// session it references). The graph mapper turns each one into a
/// Cypher edge, keyed by `(from, relation, to)` so re-classifying
/// the same event never inflates the edge count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedRelation {
    /// Source entity. Use the literal `"this_event"` to anchor
    /// the relation at the event itself; otherwise reference a
    /// previously-emitted [`ExtractedEntity::identifier`].
    pub from: String,
    /// Relation label drawn from a controlled vocabulary so the
    /// graph schema stays inspectable:
    /// `REFERENCES` (event mentions entity) |
    /// `IMPLEMENTS` (event implements decision/spec) |
    /// `FIXES` (event fixes a bug / file / regression) |
    /// `DISCUSSES` (event topic-level discusses a concept) |
    /// `DEFINES` (event introduces / defines an entity) |
    /// `DEPENDS_ON` (event depends on a file / decision /
    /// component) | `SUPERSEDES` (decision-level supersession) |
    /// `OBSERVED_IN` (entity surfaced inside this event).
    pub relation: String,
    /// Target entity identifier — must match an
    /// [`ExtractedEntity::identifier`] in the same record.
    pub to: String,
}

/// Which backend produced a [`ClassifierOutput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierSource {
    /// Haiku invoked via Claude Code CLI.
    HaikuCli,
    /// Haiku invoked via the Anthropic SDK.
    HaikuSdk,
    /// Content-hash cache hit.
    Cache,
    /// Rule-based fallback (budget halt or offline).
    StaticFallback,
}

impl ClassifierSource {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            ClassifierSource::HaikuCli => "haiku_cli",
            ClassifierSource::HaikuSdk => "haiku_sdk",
            ClassifierSource::Cache => "cache",
            ClassifierSource::StaticFallback => "static_fallback",
        }
    }
}

/// Classifier invocation mode (selected per worker process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClassifierMode {
    /// Invoke Claude Code CLI (`claude -p ...`).
    Cli,
    /// Invoke the Anthropic SDK directly.
    Sdk,
}

/// Input to a [`Classifier`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentInput {
    /// Source event id.
    pub event_id: String,
    /// Coarse event kind (from the envelope).
    pub kind: Kind,
    /// Pre-redaction content hash — the cache key.
    pub content_hash: String,
    /// Post-redaction payload the classifier sees.
    pub redacted_payload: Value,
    /// Optional repo context hint.
    pub context_repo: Option<String>,
}

/// Output of a [`Classifier`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierOutput {
    /// Echo of the input event id.
    pub event_id: String,
    /// More precise kind (e.g. `git_push`), when the classifier can refine.
    pub kind_refinement: Option<String>,
    /// Multi-label topics drawn from the controlled vocab.
    pub topics: Vec<String>,
    /// Severity label.
    pub severity: Severity,
    /// PII risk.
    pub pii_risk: PiiRisk,
    /// Extra redaction suggestions.
    pub redaction_suggestions: Vec<RedactionSuggestion>,
    /// Short summary; mandatory if redacted payload >4 KB.
    pub summary: Option<String>,
    /// Typed entities Sonnet identified inside the event — file
    /// paths, decision ids, repos, topics, concepts. The graph
    /// mapper turns each one into a Nexus node, deduplicated
    /// across all events that mention it. Empty for the static
    /// fallback (it can't extract entities by rules).
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    /// Semantic edges Sonnet identified between this event and
    /// the entities it referenced. The graph mapper turns each
    /// one into a typed edge so a query like "what implements
    /// DEC-0042?" can traverse the IMPLEMENTS edge rather than
    /// hoping for a keyword hit.
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
    /// Which backend produced this record.
    pub source: ClassifierSource,
    /// Prompt template version (`v1`, …).
    pub prompt_version: String,
    /// Model id (`claude-haiku-4-5`, `static-v1`, …).
    pub model: String,
    /// Wall-clock latency to produce this record.
    pub latency_ms: u32,
    /// Input token count (0 for cache / static fallback).
    pub tokens_in: u32,
    /// Output token count (0 for cache / static fallback).
    pub tokens_out: u32,
}

/// Classifier trait. Implementations may batch-call a backend or shortcut via a cache.
#[async_trait]
pub trait Classifier: Send + Sync {
    /// Classify a batch of enrichment inputs; order of the result must match the input order.
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, super::errors::ClassifierError>;
}

#[async_trait]
impl<T: Classifier + ?Sized> Classifier for Box<T> {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, super::errors::ClassifierError> {
        (**self).classify_batch(events).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn severity_as_str_round_trips() {
        assert_eq!(Severity::Info.as_str(), "info");
        assert_eq!(Severity::Notable.as_str(), "notable");
        assert_eq!(Severity::Critical.as_str(), "critical");
    }

    #[test]
    fn severity_serde_uses_lowercase() {
        for (s, lit) in [
            (Severity::Info, "\"info\""),
            (Severity::Notable, "\"notable\""),
            (Severity::Critical, "\"critical\""),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), lit);
            let back: Severity = serde_json::from_str(lit).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn pii_risk_as_str_and_serde() {
        for (s, lit) in [
            (PiiRisk::Low, "\"low\""),
            (PiiRisk::Medium, "\"medium\""),
            (PiiRisk::High, "\"high\""),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), lit);
            let back: PiiRisk = serde_json::from_str(lit).unwrap();
            assert_eq!(back, s);
        }
        assert_eq!(PiiRisk::Low.as_str(), "low");
        assert_eq!(PiiRisk::Medium.as_str(), "medium");
        assert_eq!(PiiRisk::High.as_str(), "high");
    }

    #[test]
    fn classifier_source_as_str_and_serde() {
        for (s, lit) in [
            (ClassifierSource::HaikuCli, "\"haiku_cli\""),
            (ClassifierSource::HaikuSdk, "\"haiku_sdk\""),
            (ClassifierSource::Cache, "\"cache\""),
            (ClassifierSource::StaticFallback, "\"static_fallback\""),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), lit);
            let back: ClassifierSource = serde_json::from_str(lit).unwrap();
            assert_eq!(back, s);
            assert_eq!(s.as_str(), lit.trim_matches('"'));
        }
    }

    #[test]
    fn classifier_mode_serde_lowercase() {
        for (m, lit) in [
            (ClassifierMode::Cli, "\"cli\""),
            (ClassifierMode::Sdk, "\"sdk\""),
        ] {
            assert_eq!(serde_json::to_string(&m).unwrap(), lit);
            let back: ClassifierMode = serde_json::from_str(lit).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn extracted_entity_label_skipped_when_none() {
        let e = ExtractedEntity {
            entity_type: "decision".into(),
            identifier: "DEC-0042".into(),
            label: None,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(!s.contains("label"));
        let with_label = ExtractedEntity {
            label: Some("ADR-42".into()),
            ..e.clone()
        };
        let s2 = serde_json::to_string(&with_label).unwrap();
        assert!(s2.contains("\"label\":\"ADR-42\""));
    }

    #[test]
    fn extracted_relation_round_trips() {
        let r = ExtractedRelation {
            from: "this_event".into(),
            relation: "IMPLEMENTS".into(),
            to: "DEC-0042".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ExtractedRelation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn enrichment_input_round_trips() {
        let i = EnrichmentInput {
            event_id: "01TEST".into(),
            kind: Kind::Turn,
            content_hash: "sha256:abc".into(),
            redacted_payload: json!({"hi": 1}),
            context_repo: Some("repo".into()),
        };
        let json = serde_json::to_string(&i).unwrap();
        let back: EnrichmentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_id, i.event_id);
        assert_eq!(back.kind, Kind::Turn);
    }

    #[test]
    fn classifier_output_defaults_for_entities_relations() {
        // Missing entities/relations in input JSON should deserialise
        // as empty vecs (the `#[serde(default)]` attr).
        let raw = serde_json::json!({
            "event_id": "01X",
            "kind_refinement": null,
            "topics": [],
            "severity": "info",
            "pii_risk": "low",
            "redaction_suggestions": [],
            "summary": null,
            "source": "static_fallback",
            "prompt_version": "v1",
            "model": "static-v1",
            "latency_ms": 0,
            "tokens_in": 0,
            "tokens_out": 0,
        });
        let out: ClassifierOutput = serde_json::from_value(raw).unwrap();
        assert!(out.entities.is_empty());
        assert!(out.relations.is_empty());
    }

    // Cover the Box<dyn Classifier> blanket impl with a deterministic
    // in-memory implementation that returns a static-fallback record
    // per input.
    struct StaticEchoClassifier;
    #[async_trait]
    impl Classifier for StaticEchoClassifier {
        async fn classify_batch(
            &self,
            events: &[EnrichmentInput],
        ) -> Result<Vec<ClassifierOutput>, super::super::errors::ClassifierError> {
            Ok(events
                .iter()
                .map(|e| ClassifierOutput {
                    event_id: e.event_id.clone(),
                    kind_refinement: None,
                    topics: vec![],
                    severity: Severity::Info,
                    pii_risk: PiiRisk::Low,
                    redaction_suggestions: vec![],
                    summary: None,
                    entities: Vec::new(),
                    relations: Vec::new(),
                    source: ClassifierSource::StaticFallback,
                    prompt_version: "v1".into(),
                    model: "static-echo".into(),
                    latency_ms: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn box_dyn_classifier_dispatches_through_blanket_impl() {
        let echo: Box<dyn Classifier> = Box::new(StaticEchoClassifier);
        let inp = EnrichmentInput {
            event_id: "X".into(),
            kind: Kind::Turn,
            content_hash: "h".into(),
            redacted_payload: json!({}),
            context_repo: None,
        };
        let out = echo.classify_batch(&[inp]).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event_id, "X");
        assert_eq!(out[0].source, ClassifierSource::StaticFallback);
    }
}
