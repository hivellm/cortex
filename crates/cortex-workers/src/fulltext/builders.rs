//! Per-kind document builders.
//!
//! Spec 08 §Per-kind document builder calls for one pure builder per
//! event family — adding a new `Kind` variant is a compile error until
//! a builder exists, so the dispatcher's exhaustive match catches drift
//! at build time rather than at runtime.
//!
//! Each builder is pure (no I/O) and trivially unit-testable. The
//! dispatcher pulls the shared core fields out of [`EnrichedEvent`]
//! and lets the per-kind helper layer on `ext.<family>` extensions.

use std::collections::BTreeMap;

use cortex_classifier::{ClassifierOutput, PiiRisk, Severity};
use cortex_core::events::{
    AgentCall, AnalysisPayload, ArtifactPayload, DecisionPayload, Kind, KnowledgePayload,
    LawViolationPayload, LearningPayload, MemoryPayload, ToolCall, Turn,
};
use serde_json::{json, Value};

use super::body::{select_body, BodySource};
use super::document::{bootstrap_doc_id, compute_path_prefixes, live_doc_id, Document};
use crate::embedder::EnrichedEvent;

/// Maximum length of the `title` field. Spec 08 §Document schema:
/// "first 80 chars otherwise".
pub const TITLE_MAX_CHARS: usize = 80;

/// Outcome of running [`build_doc`] on one event. The builder may
/// legitimately decline the event (`Skipped`) when redaction strips
/// the whole body and the classifier produced no summary.
///
/// `Ready` boxes its [`Document`] payload so the discriminant size of
/// the enum stays small — a [`Document`] carries multi-string fields
/// plus an `ext` map, so leaving it unboxed bloats every `Result` and
/// `Option<BuildOutcome>` to ~600 B which clippy flags as a smell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    /// Document ready for upsert.
    Ready(Box<Document>),
    /// Spec 08 §Body selection rule 3: empty body + no summary; bump
    /// the `fulltext.skipped_empty` counter and drop the event.
    Skipped,
}

/// Build the full-text document for one enriched event.
///
/// `bootstrap` selects the doc-id rule per spec 08 §Identity:
/// `event_id` for live, `bootstrap:<repo>:<path>:<hash>` for backfill.
pub fn build_doc(event: &EnrichedEvent, bootstrap: bool, max_body_bytes: usize) -> BuildOutcome {
    let (raw, summary_override) = extract_payload_text(event);
    let summary = event
        .classifier
        .summary
        .as_deref()
        .or(summary_override.as_deref());
    let chosen = select_body(&raw, summary, max_body_bytes);
    if matches!(chosen.source, BodySource::Empty) {
        return BuildOutcome::Skipped;
    }

    let title = derive_title(event, &raw);
    let id = compute_doc_id(event, bootstrap);
    let kind_label = kind_label(event.kind);
    let language = detect_language(event);

    // Phase6g §2 — surface write-side bugs where every read-side
    // projection field comes back empty. The lane's read-side
    // projection (kind-aware `body > summary > title` for
    // `kind=artifact`, or the inverted chain for curated kinds)
    // produces an empty `LaneHit.text` in this case, and the
    // orchestrator's degenerate-hit filter then drops the row from
    // the bundle entirely. Logging here lets operators identify
    // which upstream payloads caused the silent drop without
    // changing write semantics — `BodySource::Empty` is the only
    // path that returns early; this branch fires when `select_body`
    // produced text that is neither curated nor displayable.
    if chosen.body.is_empty() && summary.unwrap_or("").is_empty() && title.is_empty() {
        tracing::warn!(
            event_id = %event.event_id,
            kind = kind_label,
            content_hash = %event.content_hash,
            "fulltext doc has no body, summary, or title — read-side projection will be empty"
        );
    }

    let path_prefixes = event
        .context_path
        .as_deref()
        .map(compute_path_prefixes)
        .unwrap_or_default();
    let mut doc = Document {
        id,
        event_id: event.event_id.clone(),
        kind: kind_label.to_string(),
        content_hash: event.content_hash.clone(),
        ts: 0, // Filled below if classifier latency_ms / event has a hint.
        repo: event.context_repo.clone(),
        path: event.context_path.clone(),
        topics: event.classifier.topics.clone(),
        severity: severity_label(event.classifier.severity).to_string(),
        pii_risk: pii_risk_label(event.classifier.pii_risk).to_string(),
        summary: event.classifier.summary.clone(),
        title,
        body: chosen.body,
        language,
        truncated: chosen.truncated,
        path_prefixes,
        ext: BTreeMap::new(),
    };

    apply_extensions(event, &mut doc);
    BuildOutcome::Ready(Box::new(doc))
}

fn compute_doc_id(event: &EnrichedEvent, bootstrap: bool) -> String {
    if bootstrap {
        // The bootstrap path needs `(repo, path, content_hash)`. Fall
        // back to the live id when the envelope can't supply both —
        // the bootstrap CLI populates them, but defensive handling
        // keeps replays from a partially-populated upstream usable.
        match (event.context_repo.as_deref(), event.context_path.as_deref()) {
            (Some(repo), Some(path)) => bootstrap_doc_id(repo, path, &event.content_hash),
            _ => live_doc_id(&event.event_id),
        }
    } else {
        live_doc_id(&event.event_id)
    }
}

fn extract_payload_text(event: &EnrichedEvent) -> (String, Option<String>) {
    match event.kind {
        Kind::Turn => match serde_json::from_value::<Turn>(event.redacted_payload.clone()) {
            Ok(t) => {
                let mut out = t.user_message.clone();
                if let Some(ref a) = t.assistant_message {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(a);
                }
                (out, None)
            }
            Err(_) => (fallback_text(&event.redacted_payload), None),
        },
        Kind::ToolCall => match serde_json::from_value::<ToolCall>(event.redacted_payload.clone())
        {
            Ok(tc) => (tool_call_text(&tc), Some(tc.tool_name)),
            Err(_) => (fallback_text(&event.redacted_payload), None),
        },
        Kind::AgentCall => {
            match serde_json::from_value::<AgentCall>(event.redacted_payload.clone()) {
                Ok(a) => {
                    let mut out = a.description.clone();
                    if let Some(p) = a.prompt.as_deref() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(p);
                    }
                    (out, Some(a.agent_type))
                }
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
        Kind::Memory => match serde_json::from_value::<MemoryPayload>(event.redacted_payload.clone())
        {
            Ok(m) => {
                let mut out = m.name.clone();
                if let Some(b) = m.body.as_deref() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(b);
                }
                (out, m.description)
            }
            Err(_) => (fallback_text(&event.redacted_payload), None),
        },
        Kind::Decision => {
            match serde_json::from_value::<DecisionPayload>(event.redacted_payload.clone()) {
                Ok(d) => {
                    let mut out = d.title.clone();
                    if !d.body.is_empty() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&d.body);
                    }
                    (out, None)
                }
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
        Kind::Analysis => {
            match serde_json::from_value::<AnalysisPayload>(event.redacted_payload.clone()) {
                Ok(a) => (a.question, None),
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
        Kind::LawViolation => match serde_json::from_value::<LawViolationPayload>(
            event.redacted_payload.clone(),
        ) {
            Ok(lv) => {
                let mut out = format!("{}: {}", lv.law_id, lv.message);
                if !lv.evidence.is_null() {
                    out.push('\n');
                    out.push_str(&lv.evidence.to_string());
                }
                (out, None)
            }
            Err(_) => (fallback_text(&event.redacted_payload), None),
        },
        Kind::Artifact => {
            match serde_json::from_value::<ArtifactPayload>(event.redacted_payload.clone()) {
                Ok(a) => {
                    let body = a.body.unwrap_or_default();
                    (body, None)
                }
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
        // phase10e — knowledge / learnings carry the markdown body
        // verbatim. The title goes through `derive_title` below;
        // here we just hand back the body so Meili indexes it.
        Kind::Knowledge => {
            match serde_json::from_value::<KnowledgePayload>(event.redacted_payload.clone()) {
                Ok(k) => (k.body, None),
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
        Kind::Learning => {
            match serde_json::from_value::<LearningPayload>(event.redacted_payload.clone()) {
                Ok(l) => (l.body, None),
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
    }
}

fn tool_call_text(tc: &ToolCall) -> String {
    // Pull whatever obvious text the input carries — `command`,
    // `text`, `prompt`, `path`. These are the spec-08 §Body selection
    // hints. Falls through to the JSON serialisation when none match.
    if let Some(text) = read_string_field(&tc.input, "command")
        .or_else(|| read_string_field(&tc.input, "text"))
        .or_else(|| read_string_field(&tc.input, "prompt"))
    {
        return text;
    }
    tc.input.to_string()
}

fn read_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn fallback_text(payload: &Value) -> String {
    // Last resort: serialise the redacted payload. Better to index
    // *something* searchable than skip the event entirely.
    serde_json::to_string(payload).unwrap_or_default()
}

fn derive_title(event: &EnrichedEvent, raw: &str) -> String {
    let candidate: String = match event.kind {
        Kind::Turn => raw.lines().next().unwrap_or(raw).to_string(),
        Kind::ToolCall => serde_json::from_value::<ToolCall>(event.redacted_payload.clone())
            .map(|tc| tc.tool_name)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::AgentCall => serde_json::from_value::<AgentCall>(event.redacted_payload.clone())
            .map(|a| format!("{}: {}", a.agent_type, a.description))
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::Memory => serde_json::from_value::<MemoryPayload>(event.redacted_payload.clone())
            .map(|m| m.name)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::Decision => serde_json::from_value::<DecisionPayload>(event.redacted_payload.clone())
            .map(|d| d.title)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::Analysis => serde_json::from_value::<AnalysisPayload>(event.redacted_payload.clone())
            .map(|a| a.question)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::LawViolation => {
            serde_json::from_value::<LawViolationPayload>(event.redacted_payload.clone())
                .map(|lv| format!("{}: {}", lv.law_id, lv.message))
                .unwrap_or_else(|_| event.event_id.clone())
        }
        Kind::Artifact => serde_json::from_value::<ArtifactPayload>(event.redacted_payload.clone())
            .ok()
            .and_then(|a| a.path.or_else(|| a.url.clone()))
            .or_else(|| event.context_path.clone())
            .unwrap_or_else(|| event.event_id.clone()),
        Kind::Knowledge => serde_json::from_value::<KnowledgePayload>(event.redacted_payload.clone())
            .map(|k| k.title)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::Learning => serde_json::from_value::<LearningPayload>(event.redacted_payload.clone())
            .map(|l| l.title)
            .unwrap_or_else(|_| event.event_id.clone()),
    };
    take_first_chars(&candidate, TITLE_MAX_CHARS)
}

fn take_first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn detect_language(event: &EnrichedEvent) -> Option<String> {
    if let Kind::Artifact = event.kind {
        if let Ok(a) = serde_json::from_value::<ArtifactPayload>(event.redacted_payload.clone()) {
            return a.language;
        }
    }
    None
}

fn apply_extensions(event: &EnrichedEvent, doc: &mut Document) {
    match event.kind {
        Kind::ToolCall => {
            if let Ok(tc) = serde_json::from_value::<ToolCall>(event.redacted_payload.clone()) {
                doc.ext.insert(
                    "tool_call".to_string(),
                    json!({
                        "tool_name": tc.tool_name,
                        "outcome": tc.outcome,
                        "duration_ms": tc.duration_ms,
                    }),
                );
            }
        }
        Kind::AgentCall => {
            if let Ok(a) = serde_json::from_value::<AgentCall>(event.redacted_payload.clone()) {
                doc.ext.insert(
                    "agent_call".to_string(),
                    json!({
                        "agent_type": a.agent_type,
                        "outcome": a.outcome,
                        "duration_ms": a.duration_ms,
                    }),
                );
            }
        }
        Kind::Decision => {
            if let Ok(d) = serde_json::from_value::<DecisionPayload>(event.redacted_payload.clone())
            {
                let mut payload = json!({
                    "decision_id": d.decision_id,
                    "status": d.status,
                });
                if let Some(s) = d.supersedes {
                    payload["supersedes"] = Value::String(s);
                }
                if !d.tags.is_empty() {
                    payload["tags"] = Value::Array(d.tags.into_iter().map(Value::String).collect());
                }
                doc.ext.insert("decision".to_string(), payload);
            }
        }
        Kind::LawViolation => {
            if let Ok(lv) =
                serde_json::from_value::<LawViolationPayload>(event.redacted_payload.clone())
            {
                let mut payload = json!({
                    "violation_id": lv.violation_id,
                    "law_id": lv.law_id,
                    "severity": lv.severity,
                });
                if let Some(t) = lv.tier {
                    payload["tier"] = Value::from(t);
                }
                doc.ext.insert("law_violation".to_string(), payload);
            }
        }
        Kind::Memory => {
            if let Ok(m) = serde_json::from_value::<MemoryPayload>(event.redacted_payload.clone()) {
                doc.ext.insert(
                    "memory".to_string(),
                    json!({
                        "op": m.op,
                        "memory_type": m.memory_type,
                    }),
                );
            }
        }
        Kind::Analysis => {
            if let Ok(a) = serde_json::from_value::<AnalysisPayload>(event.redacted_payload.clone())
            {
                doc.ext.insert(
                    "analysis".to_string(),
                    json!({
                        "analysis_id": a.analysis_id,
                        "status": a.status,
                    }),
                );
            }
        }
        Kind::Knowledge => {
            if let Ok(k) = serde_json::from_value::<KnowledgePayload>(
                event.redacted_payload.clone(),
            ) {
                doc.ext.insert(
                    "knowledge".to_string(),
                    json!({
                        "knowledge_id": k.knowledge_id,
                        "category": k.category,
                    }),
                );
            }
        }
        Kind::Learning => {
            if let Ok(l) = serde_json::from_value::<LearningPayload>(
                event.redacted_payload.clone(),
            ) {
                let mut payload = json!({
                    "learning_id": l.learning_id,
                });
                if let Some(t) = l.related_task {
                    payload["related_task"] = Value::String(t);
                }
                doc.ext.insert("learning".to_string(), payload);
            }
        }
        Kind::Turn | Kind::Artifact => {
            // No extension fields beyond the shared core for these
            // kinds in v1 — `Turn`'s tokens belong on a separate
            // analytics index, and `Artifact` already exposes
            // `language` at the core level.
        }
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Turn => "turn",
        Kind::ToolCall => "tool_call",
        Kind::AgentCall => "agent_call",
        Kind::Memory => "memory",
        Kind::Decision => "decision",
        Kind::Analysis => "analysis",
        Kind::LawViolation => "law_violation",
        Kind::Artifact => "artifact",
        Kind::Knowledge => "knowledge",
        Kind::Learning => "learning",
    }
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Notable => "notable",
        Severity::Critical => "critical",
    }
}

fn pii_risk_label(p: PiiRisk) -> &'static str {
    match p {
        PiiRisk::Low => "low",
        PiiRisk::Medium => "medium",
        PiiRisk::High => "high",
    }
}

#[allow(dead_code)]
fn assert_classifier_output_used(_o: &ClassifierOutput) {}
