//! Rule-based [`Classifier`] used as offline / budget-halted fallback.
//!
//! Keyed off the envelope kind, the tool name, and regex hits over the
//! redacted payload. Every rule is pure and deterministic so the output is
//! reproducible and testable without a network.

use super::errors::ClassifierError;
use super::types::{
    Classifier, ClassifierOutput, ClassifierSource, EnrichmentInput, PiiRisk, SensitivityOutput,
    Severity,
};
use async_trait::async_trait;
use cortex_core::events::Kind;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::time::Instant;

/// Phase21 §3.3 — sensitivity keyword rules.
///
/// Each entry maps a regex over the flat JSON payload to `(min_level, compartment)`.
/// Multiple rules can fire; level = max of all fired levels, compartments = union.
/// Ordered most-to-least-restrictive so the table is self-documenting.
static SENSITIVITY_RULES: Lazy<Vec<(Regex, u8, &'static str)>> = Lazy::new(|| {
    vec![
        // restricted (3) — security vulnerability / exploit material.
        (
            Regex::new(
                r"(?i)\b(pentest|exploit|CVE-\d{4}|zero.?day|vuln(erabilit|erable)|poc.exploit|payload\.exec)\b",
            )
            .unwrap(),
            3,
            "security",
        ),
        // confidential (2) — financial signals.
        (
            Regex::new(
                r"(?i)\b(revenue|salary|payroll|invoice|billing|credit.?card|bank.?account|quarterly.?earnings)\b",
            )
            .unwrap(),
            2,
            "financial",
        ),
        // confidential (2) — HR signals.
        (
            Regex::new(
                r"(?i)\b(performance.?review|termination.?notice|disciplinary|medical.?leave|hr.?record|employee.?file)\b",
            )
            .unwrap(),
            2,
            "hr",
        ),
        // confidential (2) — legal-privilege signals.
        (
            Regex::new(
                r"(?i)\b(attorney.?client|privileged.?communic|litigation|settlement.?agreement|nda|non.?disclosure)\b",
            )
            .unwrap(),
            2,
            "legal",
        ),
        // confidential (2) — customer PII signals.
        (
            Regex::new(
                r"(?i)\b(ssn|social.?security.?number|passport.?number|date.?of.?birth|home.?address|customer.?pii)\b",
            )
            .unwrap(),
            2,
            "customer_pii",
        ),
        // internal (1) — generic PII escalation (redacted tokens, email addresses, auth paths).
        (
            Regex::new(r"(?i)\[REDACTED:|@[a-z0-9.-]+\.[a-z]{2,}|/Users/[^/]+/|C:\\Users\\").unwrap(),
            1,
            "customer_pii",
        ),
        // internal (1) — API/secret material that the redactor didn't catch.
        (
            Regex::new(r"(?i)\b(api.?key|secret.?key|bearer.?token|private.?key|access.?token)\b").unwrap(),
            1,
            "security",
        ),
    ]
});

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

            let flat = payload_to_string(&input.redacted_payload);
            let sensitivity = detect_sensitivity(&flat);

            out.push(ClassifierOutput {
                event_id: input.event_id.clone(),
                kind_refinement,
                topics,
                severity,
                pii_risk,
                redaction_suggestions: Vec::new(),
                // phase26c §1.2 — deterministic template summary so
                // the fulltext worker has a non-null body candidate
                // for oversize payloads and the vector embedder has
                // a readable string to embed. No LLM required; the
                // previous `"static summary: N chars"` marker that
                // was blindly copied into the Meilisearch body has
                // been replaced with a real `{kind} in {location}:
                // {snippet}` string.
                summary: Some(static_summary(
                    &input.kind,
                    &input.redacted_payload,
                    input.context_repo.as_deref(),
                )),
                entities: Vec::new(),
                relations: Vec::new(),
                sensitivity,
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

/// Build the deterministic summary for the Static classifier output.
/// Format: `"{kind} in {location}: {first 120 chars of payload text}"`.
/// The previous path emitted `"static summary: N chars"` which destroyed
/// fulltext search. This template is readable and embeddable without an LLM.
fn static_summary(kind: &Kind, payload: &Value, context_repo: Option<&str>) -> String {
    // Kind label via its snake_case serde representation.
    let kind_raw = serde_json::to_string(kind).unwrap_or_default();
    let kind_str = kind_raw.trim_matches('"');

    // Prefer an explicit `path` field in the payload; fall back to context_repo.
    let location = payload
        .get("path")
        .and_then(|v| v.as_str())
        .or(context_repo)
        .unwrap_or("unknown");

    // phase26f §2.1 — extract a clean, human-readable per-kind snippet
    // instead of the raw flattened JSON (the old `payload_to_string`
    // path produced `{"text":...}`-style noise that polluted the Meili
    // summary + keyword lane). Mirrors the embedder's `nl_projection`
    // field selection so the summary reads like prose.
    let nl = nl_summary_snippet(kind, payload);
    let snippet: String = nl.chars().take(160).collect();

    format!("{kind_str} in {location}: {snippet}")
}

/// phase26f §2.1 — pull a human-readable snippet from the payload's
/// per-kind text fields (no raw JSON). Falls back to generic
/// `content`/`text`/`body` fields, then to an empty string. Whitespace
/// is collapsed so the summary stays single-line.
fn nl_summary_snippet(kind: &Kind, payload: &Value) -> String {
    let field = |k: &str| {
        payload
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let join = |parts: &[Option<&str>]| -> String {
        parts
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .join(" — ")
    };
    let raw = match kind {
        Kind::Turn => join(&[field("user_message"), field("assistant_message")]),
        Kind::ToolCall => {
            let tool = field("tool_name").unwrap_or("tool");
            let input = payload
                .get("input")
                .map(summarize_value)
                .unwrap_or_default();
            if input.is_empty() {
                tool.to_string()
            } else {
                format!("{tool}: {input}")
            }
        }
        Kind::AgentCall => join(&[field("description"), field("prompt")]),
        Kind::Memory => join(&[field("name"), field("body"), field("description")]),
        Kind::Decision => join(&[field("title"), field("status"), field("body")]),
        Kind::Analysis => join(&[field("question"), field("title")]),
        Kind::Law => join(&[field("title"), field("body")]),
        Kind::LawViolation => join(&[field("law_id"), field("message")]),
        Kind::Knowledge | Kind::Learning => join(&[field("title"), field("body")]),
        Kind::Consolidation => join(&[field("summary_markdown"), field("title")]),
        Kind::TopicCard => join(&[field("synthesis_markdown"), field("title")]),
        _ => String::new(),
    };
    // Generic fallback when the per-kind fields were absent.
    let chosen = if raw.is_empty() {
        join(&[field("body"), field("content"), field("text")])
    } else {
        raw
    };
    // Collapse all whitespace runs to single spaces for a clean line.
    chosen.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Compact, JSON-free rendering of a tool-call `input` value: object
/// values become `k=v` pairs, scalars print directly. Keeps the
/// summary readable without embedding raw JSON braces.
fn summarize_value(v: &Value) -> String {
    match v {
        Value::Object(map) => map
            .iter()
            .take(4)
            .map(|(k, val)| {
                let vs = match val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let vs: String = vs.chars().take(40).collect();
                format!("{k}={vs}")
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
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
        Kind::Law => {
            topics.push("law".into());
            topics.push("governance".into());
            if let Some(s) = payload.get("severity").and_then(|v| v.as_str()) {
                severity = match s {
                    "critical" => Severity::Critical,
                    "notable" => Severity::Notable,
                    _ => Severity::Info,
                };
            }
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
        // phase11r §3.2 — TopicCards carry the `topic_cards`
        // canonical topic + the topic_slug as a topic so the
        // dashboard can filter by topic. Severity stays at info.
        Kind::TopicCard => {
            topics.push("topic_cards".into());
            if let Some(slug) = payload.get("topic_slug").and_then(|v| v.as_str()) {
                topics.push(slug.to_string());
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

/// Detect sensitivity from payload content signals (phase21 §3.3).
///
/// Scans the flat payload for keyword patterns in [`SENSITIVITY_RULES`].
/// Returns the escalated `SensitivityOutput` (`level = max` + compartment union).
/// Always escalate-only: the returned level/compartments may be merged
/// with a declared floor via [`super::types::merge_sensitivity`].
pub fn detect_sensitivity(flat: &str) -> SensitivityOutput {
    let mut level: u8 = 0;
    let mut compartments: Vec<&'static str> = Vec::new();

    for (re, rule_level, compartment) in SENSITIVITY_RULES.iter() {
        if re.is_match(flat) {
            if *rule_level > level {
                level = *rule_level;
            }
            if !compartments.contains(compartment) {
                compartments.push(compartment);
            }
        }
    }

    SensitivityOutput {
        level,
        compartments: compartments.iter().map(|s| s.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::types::merge_sensitivity;

    // ---- detect_sensitivity unit tests (phase21 §3.3) ---- //

    #[test]
    fn detect_sensitivity_public_for_clean_payload() {
        let out = detect_sensitivity("ordinary text with no sensitive keywords");
        assert_eq!(out.level, 0, "clean payload must be public");
        assert!(
            out.compartments.is_empty(),
            "clean payload has no compartments"
        );
    }

    #[test]
    fn detect_sensitivity_security_keywords_yield_restricted() {
        let out = detect_sensitivity("zero-day exploit found in CVE-2024-9999 poc.exploit");
        assert_eq!(out.level, 3, "security exploit keywords → restricted");
        assert!(
            out.compartments.contains(&"security".to_string()),
            "compartments must include security; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_financial_keywords_yield_confidential() {
        let out = detect_sensitivity("quarterly earnings and revenue figures for billing cycle");
        assert_eq!(out.level, 2, "financial keywords → confidential");
        assert!(
            out.compartments.contains(&"financial".to_string()),
            "compartments must include financial; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_hr_keywords_yield_confidential() {
        let out = detect_sensitivity("performance review and disciplinary action in hr record");
        assert_eq!(out.level, 2, "HR keywords → confidential");
        assert!(
            out.compartments.contains(&"hr".to_string()),
            "compartments must include hr; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_legal_keywords_yield_confidential() {
        let out = detect_sensitivity("attorney-client privileged communication under NDA");
        assert_eq!(out.level, 2, "legal keywords → confidential");
        assert!(
            out.compartments.contains(&"legal".to_string()),
            "compartments must include legal; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_customer_pii_keywords_yield_confidential() {
        let out = detect_sensitivity("SSN 123-45-6789 and passport number found in customer PII");
        assert_eq!(out.level, 2, "customer PII keywords → confidential");
        assert!(
            out.compartments.contains(&"customer_pii".to_string()),
            "compartments must include customer_pii; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_redacted_token_yields_internal() {
        let out = detect_sensitivity("[REDACTED:api_key] user@example.com accessed /Users/alice/");
        assert_eq!(out.level, 1, "redacted token / email → internal");
        assert!(
            out.compartments.contains(&"customer_pii".to_string()),
            "compartments must include customer_pii; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_api_key_material_yields_internal() {
        let out = detect_sensitivity("api_key=sk-abc123 bearer token passed in header");
        assert_eq!(out.level, 1, "api key / bearer token → internal");
        assert!(
            out.compartments.contains(&"security".to_string()),
            "compartments must include security; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_multiple_rules_max_level_union_compartments() {
        // Both financial (2) and security exploit (3) fire — level must be 3,
        // compartments must be the union {"security", "financial"}.
        let out =
            detect_sensitivity("CVE-2024-1234 zero-day in our quarterly earnings payroll system");
        assert_eq!(out.level, 3, "max of financial+security → restricted");
        assert!(
            out.compartments.contains(&"security".to_string()),
            "security compartment must be present; got {:?}",
            out.compartments
        );
        assert!(
            out.compartments.contains(&"financial".to_string()),
            "financial compartment must be present; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn detect_sensitivity_no_duplicate_compartments() {
        // Only the security compartment fires — must appear exactly once.
        let out = detect_sensitivity("CVE-2024-1234 zero-day pentest payload.exec exploit");
        assert_eq!(
            out.compartments.iter().filter(|c| *c == "security").count(),
            1,
            "security compartment must appear exactly once; got {:?}",
            out.compartments
        );
    }

    // ---- merge_sensitivity unit tests (phase21 §3.4 precursor) ---- //

    #[test]
    fn merge_sensitivity_escalate_only_takes_max_level() {
        let a = SensitivityOutput {
            level: 1,
            compartments: vec!["security".into()],
        };
        let b = SensitivityOutput {
            level: 3,
            compartments: vec!["financial".into()],
        };
        let merged = merge_sensitivity(a, b);
        assert_eq!(merged.level, 3, "merge must take the max level");
    }

    #[test]
    fn merge_sensitivity_compartments_are_unioned() {
        let a = SensitivityOutput {
            level: 2,
            compartments: vec!["hr".into(), "legal".into()],
        };
        let b = SensitivityOutput {
            level: 1,
            compartments: vec!["legal".into(), "financial".into()],
        };
        let merged = merge_sensitivity(a, b);
        assert_eq!(merged.level, 2, "level = max(2, 1) = 2");
        let mut comps = merged.compartments.clone();
        comps.sort();
        assert_eq!(
            comps,
            vec!["financial", "hr", "legal"],
            "compartments = union, no duplicates"
        );
    }

    #[test]
    fn merge_sensitivity_public_with_public_stays_public() {
        let a = SensitivityOutput::default();
        let b = SensitivityOutput::default();
        let merged = merge_sensitivity(a, b);
        assert_eq!(merged.level, 0);
        assert!(merged.compartments.is_empty());
    }

    #[test]
    fn merge_sensitivity_commutative_level() {
        let a = SensitivityOutput {
            level: 3,
            compartments: vec![],
        };
        let b = SensitivityOutput {
            level: 1,
            compartments: vec![],
        };
        assert_eq!(merge_sensitivity(a.clone(), b.clone()).level, 3);
        assert_eq!(
            merge_sensitivity(b, a).level,
            3,
            "merge must be commutative for level"
        );
    }

    // ---- redaction ordering tests (phase21 §3.5) ---- //

    #[test]
    fn redacted_token_in_payload_still_elevates_to_internal_customer_pii() {
        // After the redactor runs, the raw secret is gone but [REDACTED:...] remains.
        // The sensitivity detector must still see the token and escalate to internal.
        let redacted_flat = r#"{"body": "[REDACTED:generic_env_secret] user data here"}"#;
        let out = detect_sensitivity(redacted_flat);
        assert!(
            out.level >= 1,
            "redacted token must elevate level to at least internal; got {}",
            out.level
        );
        assert!(
            out.compartments.contains(&"customer_pii".to_string()),
            "redacted token must set customer_pii compartment; got {:?}",
            out.compartments
        );
    }

    #[test]
    fn raw_pii_removed_by_redaction_does_not_trigger_customer_pii_alone() {
        // Verify the converse: a payload with the raw SSN phrase (no [REDACTED:])
        // hits the customer_pii rule via the ssn keyword.
        let raw_pii = "SSN 123-45-6789 is present";
        let out = detect_sensitivity(raw_pii);
        assert_eq!(out.level, 2, "raw SSN → confidential");
        assert!(
            out.compartments.contains(&"customer_pii".to_string()),
            "raw SSN → customer_pii compartment"
        );
    }

    #[test]
    fn redaction_and_classification_do_not_downgrade_each_other() {
        // Both a security exploit keyword AND a redacted token fire.
        // Neither mechanism should lower the final level — result must be max.
        let payload = "CVE-2024-9999 zero-day exploit [REDACTED:bearer_token] in request";
        let out = detect_sensitivity(payload);
        assert_eq!(out.level, 3, "security exploit must dominate → restricted");
        assert!(
            out.compartments.contains(&"security".to_string()),
            "security compartment present"
        );
        assert!(
            out.compartments.contains(&"customer_pii".to_string()),
            "customer_pii also present from REDACTED token"
        );
    }

    // ---- static_classifier summary test (phase26c §1.2) ---- //

    /// phase26c §1.2 — the static path now emits a deterministic
    /// `{kind} in {location}: {snippet}` summary so oversize events
    /// have a readable body candidate for the fulltext worker.
    /// Guard: the old `"static summary: N chars"` garbage marker must
    /// never come back.
    #[tokio::test]
    async fn static_classifier_emits_deterministic_template_summary() {
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
        let summary = out[0].summary.as_deref().expect("summary must be Some");
        assert!(
            summary.starts_with("artifact in"),
            "summary must start with 'artifact in'; got: {summary:?}"
        );
        assert!(
            !summary.contains("static summary"),
            "old garbage marker must not reappear; got: {summary:?}"
        );
        // Template caps snippet at 160 chars — total length is bounded.
        assert!(
            summary.len() <= 200,
            "summary must be short; got {} chars",
            summary.len()
        );
    }

    #[test]
    fn nl_summary_snippet_is_clean_prose_not_raw_json() {
        // phase26f §2.1 — a Turn summary reads the message fields as
        // prose, never the raw JSON braces.
        let turn = serde_json::json!({
            "user_message": "how do I add an index",
            "assistant_message": "use CREATE INDEX on the prop",
        });
        let s = nl_summary_snippet(&Kind::Turn, &turn);
        assert_eq!(s, "how do I add an index — use CREATE INDEX on the prop");
        assert!(!s.contains('{') && !s.contains("\":"), "no raw JSON: {s}");

        // A ToolCall renders `tool: k=v` pairs, not a JSON object.
        let tc = serde_json::json!({
            "tool_name": "Bash",
            "input": { "command": "cargo test", "timeout": 60 },
        });
        let s = nl_summary_snippet(&Kind::ToolCall, &tc);
        assert!(s.starts_with("Bash:"), "tool prefix: {s}");
        assert!(s.contains("command=cargo test"), "input rendered: {s}");
        assert!(!s.contains('{'), "no raw JSON braces: {s}");

        // Generic fallback when no per-kind fields are present.
        let other = serde_json::json!({ "text": "  plain   body  text " });
        let s = nl_summary_snippet(&Kind::Artifact, &other);
        assert_eq!(s, "plain body text", "whitespace collapsed, text used");
    }

    #[test]
    fn nl_summary_snippet_covers_decision_knowledge_and_learning_kinds() {
        // phase26f §2.1 / spec-05 "Summary contract" — Decision reads
        // title/status/body as prose, never the raw JSON object.
        let decision = serde_json::json!({
            "title": "Adopt stable vector ids",
            "status": "accepted",
            "body": "Avoids re-embedding on every bootstrap replay.",
        });
        let s = nl_summary_snippet(&Kind::Decision, &decision);
        assert_eq!(
            s,
            "Adopt stable vector ids — accepted — Avoids re-embedding on every bootstrap replay."
        );
        assert!(!s.contains('{') && !s.contains("\":"), "no raw JSON: {s}");

        // Knowledge and Learning both project title/body.
        let knowledge = serde_json::json!({
            "title": "wiremock is already a dev-dependency",
            "body": "cortex-api and cortex-workers both pin wiremock 0.6.",
        });
        let s = nl_summary_snippet(&Kind::Knowledge, &knowledge);
        assert_eq!(
            s,
            "wiremock is already a dev-dependency — cortex-api and cortex-workers both pin wiremock 0.6."
        );
        assert!(!s.contains('{'), "no raw JSON braces: {s}");

        let learning = serde_json::json!({
            "title": "env-var globals race across concurrent tests",
            "body": "Prefer constructor injection over process env for test doubles.",
        });
        let s = nl_summary_snippet(&Kind::Learning, &learning);
        assert_eq!(
            s,
            "env-var globals race across concurrent tests — Prefer constructor injection over process env for test doubles."
        );
        assert!(!s.contains('{'), "no raw JSON braces: {s}");
    }

    #[test]
    fn nl_summary_snippet_covers_agent_call_and_memory_kinds() {
        // AgentCall projects description/prompt.
        let agent_call = serde_json::json!({
            "description": "spawn implementer",
            "prompt": "implement §5.2 tests",
        });
        let s = nl_summary_snippet(&Kind::AgentCall, &agent_call);
        assert_eq!(s, "spawn implementer — implement §5.2 tests");
        assert!(!s.contains('{'), "no raw JSON braces: {s}");

        // Memory projects name/body/description.
        let memory = serde_json::json!({
            "name": "phase26f",
            "body": "retrieval NL quality follow-ups",
        });
        let s = nl_summary_snippet(&Kind::Memory, &memory);
        assert_eq!(s, "phase26f — retrieval NL quality follow-ups");
        assert!(!s.contains('{'), "no raw JSON braces: {s}");
    }

    #[test]
    fn static_summary_wraps_decision_kind_without_raw_json() {
        // phase26f §2.1 — the public `static_summary` wrapper (used by
        // `classify_batch`) must carry the same clean prose through to
        // the final `{kind} in {location}: {snippet}` string; this is
        // the black-box path the fulltext worker + embedder consume.
        let decision = serde_json::json!({
            "title": "Adopt stable vector ids",
            "status": "accepted",
            "body": "Avoids re-embedding on every bootstrap replay.",
        });
        let summary = static_summary(&Kind::Decision, &decision, Some("Cortex"));
        assert!(
            summary.starts_with("decision in Cortex: Adopt stable vector ids"),
            "summary must lead with kind/location/title; got: {summary:?}"
        );
        assert!(
            !summary.contains('{') && !summary.contains("\":"),
            "no raw JSON leaking into the wrapped summary: {summary:?}"
        );
    }
}
