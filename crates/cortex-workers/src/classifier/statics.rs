//! Rule-based [`Classifier`] used as offline / budget-halted fallback.
//!
//! Keyed off the envelope kind, the tool name, and regex hits over the
//! redacted payload. Every rule is pure and deterministic so the output is
//! reproducible and testable without a network.

use super::errors::ClassifierError;
use super::types::{
    Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput, PiiRisk, Severity,
};
use async_trait::async_trait;
use cortex_core::events::Kind;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::time::Instant;

/// Keyword → topic hits (used across tool-call inputs and messages).
static KEYWORD_TOPICS: Lazy<Vec<(Regex, &'static [&'static str])>> = Lazy::new(|| {
    vec![
        (
            Regex::new(r"(?i)\bgit\s+push\b").unwrap(),
            &["git", "git_push"],
        ),
        (
            Regex::new(r"(?i)\bgit\s+commit\b").unwrap(),
            &["git", "git_commit"],
        ),
        (
            Regex::new(r"(?i)\brefactor(ing|ed)?\b").unwrap(),
            &["refactor"],
        ),
        (
            Regex::new(r"(?i)\bcargo\s+test\b").unwrap(),
            &["test", "ci"],
        ),
        (Regex::new(r"(?i)\bpytest\b|\bjest\b").unwrap(), &["test"]),
        (
            Regex::new(r"(?i)\bdocker\b|\bcompose\b").unwrap(),
            &["deploy", "build"],
        ),
        (
            Regex::new(r"(?i)\bbench(mark)?\b").unwrap(),
            &["perf", "test"],
        ),
    ]
});

/// Static classifier.
#[derive(Debug, Default)]
pub struct StaticClassifier {
    /// Optional deployment label stamped on outputs — purely informational.
    pub deployment: Option<String>,
}

impl StaticClassifier {
    /// Build an anonymous static classifier.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Classifier for StaticClassifier {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, ClassifierError> {
        let mut out = Vec::with_capacity(events.len());
        for input in events {
            let started = Instant::now();
            let (kind_refinement, mut topics, mut severity, mut pii_risk) =
                classify_one(&input.kind, &input.redacted_payload);

            topics.sort();
            topics.dedup();

            if contains_blocking_language(&input.redacted_payload) {
                severity = Severity::Critical;
            }
            if contains_secret_language(&input.redacted_payload) {
                pii_risk = PiiRisk::High;
            }

            out.push(ClassifierOutput {
                event_id: input.event_id.clone(),
                kind_refinement,
                topics,
                severity,
                pii_risk,
                redaction_suggestions: Vec::new(),
                // No synthesised summary — the static path has no
                // model to summarise with, and the previous
                // `"static summary: N chars"` marker was being
                // copied into Meilisearch's `body` field by the
                // fulltext worker, destroying full-text search on
                // every artifact. Downstream consumers fall back to
                // the source `text` when this is `None`.
                summary: None,
                entities: Vec::new(),
                relations: Vec::new(),
                source: ClassifierSource::StaticFallback,
                prompt_version: "static-v1".into(),
                model: "static-v1".into(),
                latency_ms: started.elapsed().as_millis() as u32,
                tokens_in: 0,
                tokens_out: 0,
            });
        }
        Ok(out)
    }
}

fn classify_one(kind: &Kind, payload: &Value) -> (Option<String>, Vec<String>, Severity, PiiRisk) {
    let mut topics: Vec<String> = Vec::new();
    let mut kind_refinement: Option<String> = None;
    let (mut severity, mut pii_risk) = (Severity::Info, PiiRisk::Low);

    let flat = payload_to_string(payload);

    for (re, tags) in KEYWORD_TOPICS.iter() {
        if re.is_match(&flat) {
            for t in *tags {
                topics.push((*t).into());
            }
        }
    }

    match kind {
        Kind::ToolCall => {
            topics.push("code".into());
            if let Some(tool_name) = payload.get("tool_name").and_then(|v| v.as_str()) {
                match tool_name {
                    "Bash" => {
                        topics.push("code".into());
                        if flat.contains("git push") {
                            kind_refinement = Some("git_push".into());
                            severity = Severity::Notable;
                        } else if flat.contains("git commit") {
                            kind_refinement = Some("git_commit".into());
                        } else if flat.contains("cargo test") || flat.contains("npm test") {
                            kind_refinement = Some("test_run".into());
                        }
                    }
                    "Edit" | "Write" | "MultiEdit" => {
                        topics.push("code".into());
                        kind_refinement = Some("file_edit".into());
                        severity = Severity::Notable;
                    }
                    "Read" => {
                        topics.push("read".into());
                    }
                    _ => {}
                }
            }
            if let Some(outcome) = payload.get("outcome").and_then(|v| v.as_str()) {
                if outcome.starts_with("blocked_by_law:") {
                    severity = Severity::Critical;
                    topics.push("law".into());
                    topics.push("governance".into());
                } else if outcome == "error" {
                    severity = Severity::Notable;
                    topics.push("error".into());
                }
            }
        }
        Kind::Turn => {
            topics.push("code".into());
        }
        Kind::AgentCall => {
            topics.push("code".into());
            if let Some(o) = payload.get("outcome").and_then(|v| v.as_str()) {
                match o {
                    "error" => severity = Severity::Notable,
                    "timeout" => {
                        severity = Severity::Notable;
                        topics.push("timeout".into());
                    }
                    "cancelled" => topics.push("cancel".into()),
                    _ => {}
                }
            }
        }
        Kind::Decision => {
            topics.push("decision".into());
            severity = Severity::Notable;
        }
        Kind::Analysis => {
            topics.push("analysis".into());
            severity = Severity::Notable;
        }
        Kind::LawViolation => {
            topics.push("law".into());
            topics.push("governance".into());
            severity = Severity::Critical;
            if let Some(s) = payload.get("severity").and_then(|v| v.as_str()) {
                severity = match s {
                    "critical" => Severity::Critical,
                    "notable" => Severity::Notable,
                    _ => Severity::Info,
                };
            }
        }
        Kind::Memory => {
            topics.push("memory".into());
        }
        // phase10e — knowledge + learnings carry their own
        // canonical topic and route to dedicated collections /
        // indexes downstream. No severity bump (they are
        // reference material, not a notable event).
        Kind::Knowledge => {
            topics.push("knowledge".into());
            if let Some(cat) = payload.get("category").and_then(|v| v.as_str()) {
                topics.push(cat.to_string());
            }
        }
        Kind::Learning => {
            topics.push("learning".into());
        }
        Kind::Artifact => {
            topics.push("code".into());
            if let Some(l) = payload.get("language").and_then(|v| v.as_str()) {
                if !l.is_empty() {
                    topics.push(l.to_ascii_lowercase());
                }
            }
        }
        // Phase11j — Consolidations carry the `consolidations`
        // canonical topic + the per-grain label so the dashboard
        // can filter by grain. Severity stays at info (curated
        // material, not a notable event).
        Kind::Consolidation => {
            topics.push("consolidations".into());
            if let Some(grain) = payload.get("grain").and_then(|v| v.as_str()) {
                topics.push(grain.to_string());
            }
        }
    }

    if flat.contains("[REDACTED:") {
        pii_risk = PiiRisk::High;
    } else if contains_usernames_or_paths(&flat) {
        pii_risk = PiiRisk::Medium;
    }

    (kind_refinement, topics, severity, pii_risk)
}

fn contains_blocking_language(payload: &Value) -> bool {
    let flat = payload_to_string(payload);
    flat.contains("blocked_by_law:")
}

fn contains_secret_language(payload: &Value) -> bool {
    let flat = payload_to_string(payload);
    flat.contains("[REDACTED:")
}

fn contains_usernames_or_paths(flat: &str) -> bool {
    flat.contains('@') || flat.contains("/Users/") || flat.contains(":/") || flat.contains("C:\\")
}

fn payload_to_string(payload: &Value) -> String {
    serde_json::to_string(payload).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the static path used to emit
    /// `summary = "static summary: <N> chars"` for any payload over
    /// 4 KB. The fulltext worker copied that string into Meilisearch's
    /// `body` field, and full-text search broke for every artifact —
    /// the indexed body had no real tokens. Verify the static path
    /// now returns `summary: None` so downstream consumers fall back
    /// to the source `text`.
    #[tokio::test]
    async fn static_classifier_emits_no_summary_even_for_oversize_payloads() {
        let big = "x".repeat(20_000);
        let input = EnrichmentInput {
            event_id: "evt-static-summary".into(),
            kind: Kind::Artifact,
            content_hash: "sha256:0".into(),
            redacted_payload: serde_json::json!({ "text": big }),
            context_repo: Some("Cortex".into()),
        };
        let out = StaticClassifier::new()
            .classify_batch(std::slice::from_ref(&input))
            .await
            .expect("classify");
        assert_eq!(out.len(), 1);
        assert!(
            out[0].summary.is_none(),
            "static path must not synthesise a summary string; got: {:?}",
            out[0].summary
        );
    }
}
