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
        decision_id: None,
        decision_title: None,
        decision_status: None,
        decision_supersedes: None,
        law_id: None,
        law_severity: None,
        law_tier: None,
        turn_id: None,
        ext: BTreeMap::new(),
    };

    apply_extensions(event, &mut doc);
    apply_top_level_projection(event, &mut doc);
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
        Kind::ToolCall => {
            match serde_json::from_value::<ToolCall>(event.redacted_payload.clone()) {
                Ok(tc) => (tool_call_text(&tc), Some(tc.tool_name)),
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
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
        Kind::Memory => {
            match serde_json::from_value::<MemoryPayload>(event.redacted_payload.clone()) {
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
            }
        }
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
        Kind::LawViolation => {
            match serde_json::from_value::<LawViolationPayload>(event.redacted_payload.clone()) {
                Ok(lv) => {
                    let mut out = format!("{}: {}", lv.law_id, lv.message);
                    if !lv.evidence.is_null() {
                        out.push('\n');
                        out.push_str(&lv.evidence.to_string());
                    }
                    (out, None)
                }
                Err(_) => (fallback_text(&event.redacted_payload), None),
            }
        }
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
        // Phase11j — Consolidations index the summary body so the
        // keyword lane can match against takeaways + summary text.
        // Title goes through `derive_title` below.
        Kind::Consolidation => {
            match serde_json::from_value::<cortex_core::events::ConsolidationPayload>(
                event.redacted_payload.clone(),
            ) {
                Ok(c) => {
                    let mut body = c.summary_markdown;
                    if !c.takeaways.is_empty() {
                        body.push_str("\n\n");
                        for t in &c.takeaways {
                            body.push_str("- ");
                            body.push_str(t);
                            body.push('\n');
                        }
                    }
                    (body, None)
                }
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
        Kind::Knowledge => {
            serde_json::from_value::<KnowledgePayload>(event.redacted_payload.clone())
                .map(|k| k.title)
                .unwrap_or_else(|_| event.event_id.clone())
        }
        Kind::Learning => serde_json::from_value::<LearningPayload>(event.redacted_payload.clone())
            .map(|l| l.title)
            .unwrap_or_else(|_| event.event_id.clone()),
        Kind::Consolidation => serde_json::from_value::<cortex_core::events::ConsolidationPayload>(
            event.redacted_payload.clone(),
        )
        .map(|c| c.title)
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
            if let Ok(k) =
                serde_json::from_value::<KnowledgePayload>(event.redacted_payload.clone())
            {
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
            if let Ok(l) = serde_json::from_value::<LearningPayload>(event.redacted_payload.clone())
            {
                let mut payload = json!({
                    "learning_id": l.learning_id,
                });
                if let Some(t) = l.related_task {
                    payload["related_task"] = Value::String(t);
                }
                doc.ext.insert("learning".to_string(), payload);
            }
        }
        Kind::Consolidation => {
            if let Ok(c) = serde_json::from_value::<cortex_core::events::ConsolidationPayload>(
                event.redacted_payload.clone(),
            ) {
                doc.ext.insert(
                    "consolidation".to_string(),
                    json!({
                        "consolidation_id": c.consolidation_id,
                        "grain": match c.grain {
                            cortex_core::events::ConsolidationGrain::Session => "session",
                            cortex_core::events::ConsolidationGrain::Topic => "topic",
                            cortex_core::events::ConsolidationGrain::DecisionTrace => "decision_trace",
                        },
                        "depth": match c.depth {
                            cortex_core::events::ConsolidationDepth::Shallow => "shallow",
                            cortex_core::events::ConsolidationDepth::Deep => "deep",
                        },
                        "model": c.model,
                        "source_event_count": c.source_event_count,
                    }),
                );
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

/// Phase11k §1.2 — stamp the spec-11 lane-projection contract keys at
/// the TOP level of the Meili document. The read-side projection in
/// `crates/cortex-api/src/meili_lane.rs::project` flattens unknown
/// top-level fields into `extras_raw`, then copies them into
/// `LaneHit.extras` via `LANE_EXTRAS_KEYS`. The orchestrator's overlay
/// derivers (`derive_decisions`, `derive_laws`, `derive_similar_turns`)
/// read those keys directly off `extras` — so without this step the
/// `decisions[]` / `violations[]` overlays come back empty even when
/// the per-repo `cortex-{slug}-decisions` / `-governance` indexes are
/// fully populated. Nesting under `ext.decision.*` would force the
/// reader to flatten one level deeper, which Meili's filterable schema
/// can't express.
fn apply_top_level_projection(event: &EnrichedEvent, doc: &mut Document) {
    match event.kind {
        Kind::Decision => {
            if let Ok(d) = serde_json::from_value::<DecisionPayload>(event.redacted_payload.clone())
            {
                doc.decision_id = Some(d.decision_id);
                doc.decision_title = Some(d.title);
                doc.decision_status = Some(d.status);
                doc.decision_supersedes = d.supersedes;
            }
        }
        Kind::LawViolation => {
            if let Ok(lv) =
                serde_json::from_value::<LawViolationPayload>(event.redacted_payload.clone())
            {
                doc.law_id = Some(lv.law_id);
                doc.law_severity = Some(lv.severity);
                doc.law_tier = lv.tier.map(|t| t.to_string());
            }
        }
        Kind::Turn => {
            doc.turn_id = Some(event.event_id.clone());
        }
        Kind::ToolCall
        | Kind::AgentCall
        | Kind::Memory
        | Kind::Analysis
        | Kind::Artifact
        | Kind::Knowledge
        | Kind::Learning
        | Kind::Consolidation => {
            // No top-level projection for these kinds — their overlay
            // contracts (if any) read from `ext.<family>.*` already.
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
        Kind::Consolidation => "consolidation",
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

#[cfg(test)]
mod top_level_projection_tests {
    use super::*;
    use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_core::events::Kind;
    use serde_json::json;

    fn classifier(event_id: &str) -> ClassifierOutput {
        ClassifierOutput {
            event_id: event_id.to_string(),
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn evt(event_id: &str, kind: Kind, payload: serde_json::Value) -> EnrichedEvent {
        EnrichedEvent {
            event_id: event_id.to_string(),
            kind,
            content_hash: format!("h-{event_id}"),
            redacted_payload: payload,
            classifier: classifier(event_id),
            context_repo: None,
            context_path: None,
            parent_event_id: None,
            session_id: None,
        }
    }

    fn ready(out: BuildOutcome) -> Document {
        match out {
            BuildOutcome::Ready(d) => *d,
            BuildOutcome::Skipped => panic!("expected Ready, got Skipped"),
        }
    }

    #[test]
    fn decision_event_stamps_decision_id_title_status_supersedes() {
        // Phase11k §1.4 — the spec-11 lane projection contract reads
        // these directly off `extras_raw`. Stamp at the top level so
        // the orchestrator's `derive_decisions` overlay surfaces a
        // populated `results.decisions[]`.
        let doc = ready(build_doc(
            &evt(
                "dec-1",
                Kind::Decision,
                json!({
                    "decision_id": "ADR-0042",
                    "title": "Adopt Meili",
                    "status": "accepted",
                    "body": "We pick Meili over Lexum for v1.",
                    "supersedes": "ADR-0001",
                    "tags": []
                }),
            ),
            false,
            1024 * 1024,
        ));
        assert_eq!(doc.decision_id.as_deref(), Some("ADR-0042"));
        assert_eq!(doc.decision_title.as_deref(), Some("Adopt Meili"));
        assert_eq!(doc.decision_status.as_deref(), Some("accepted"));
        assert_eq!(doc.decision_supersedes.as_deref(), Some("ADR-0001"));
    }

    #[test]
    fn decision_event_without_supersedes_leaves_field_absent() {
        let doc = ready(build_doc(
            &evt(
                "dec-2",
                Kind::Decision,
                json!({
                    "decision_id": "ADR-0099",
                    "title": "Adopt Nexus",
                    "status": "proposed",
                    "body": "Body",
                    "tags": []
                }),
            ),
            false,
            1024 * 1024,
        ));
        assert_eq!(doc.decision_id.as_deref(), Some("ADR-0099"));
        assert!(doc.decision_supersedes.is_none());
        // `skip_serializing_if = "Option::is_none"` keeps the field
        // out of the wire payload entirely — pin that contract too.
        let raw = serde_json::to_value(&doc).expect("serialise");
        assert!(raw.get("decision_supersedes").is_none());
    }

    #[test]
    fn law_violation_event_stamps_law_id_severity_tier() {
        let doc = ready(build_doc(
            &evt(
                "lv-1",
                Kind::LawViolation,
                json!({
                    "violation_id": "VIO-0001",
                    "law_id": "LAW-CORTEX-001",
                    "severity": "critical",
                    "tier": 1,
                    "message": "task sequence cherry pick",
                    "evidence": null
                }),
            ),
            false,
            1024 * 1024,
        ));
        assert_eq!(doc.law_id.as_deref(), Some("LAW-CORTEX-001"));
        assert_eq!(doc.law_severity.as_deref(), Some("critical"));
        assert_eq!(doc.law_tier.as_deref(), Some("1"));
    }

    #[test]
    fn law_violation_event_without_tier_leaves_field_absent() {
        let doc = ready(build_doc(
            &evt(
                "lv-2",
                Kind::LawViolation,
                json!({
                    "violation_id": "VIO-0002",
                    "law_id": "LAW-008",
                    "severity": "notable",
                    "message": "x",
                    "evidence": null
                }),
            ),
            false,
            1024 * 1024,
        ));
        assert!(doc.law_tier.is_none());
    }

    #[test]
    fn turn_event_stamps_turn_id_equal_to_event_id() {
        let doc = ready(build_doc(
            &evt(
                "turn-abc",
                Kind::Turn,
                json!({ "user_message": "hi", "assistant_message": "hi" }),
            ),
            false,
            1024 * 1024,
        ));
        assert_eq!(doc.turn_id.as_deref(), Some("turn-abc"));
        // Non-turn projection keys stay absent.
        assert!(doc.decision_id.is_none());
        assert!(doc.law_id.is_none());
    }

    #[test]
    fn non_governance_kinds_carry_no_top_level_projection() {
        // Memory / Knowledge / Learning / Analysis / etc. carry their
        // own overlay contracts under `ext.<family>.*` — the top-level
        // projection MUST stay absent so the JSON wire shape doesn't
        // grow null-padding.
        let doc = ready(build_doc(
            &evt(
                "mem-1",
                Kind::Memory,
                json!({
                    "memory_id": "MEM-1",
                    "name": "n",
                    "memory_type": "user",
                    "op": "save"
                }),
            ),
            false,
            1024 * 1024,
        ));
        assert!(doc.decision_id.is_none());
        assert!(doc.decision_title.is_none());
        assert!(doc.law_id.is_none());
        assert!(doc.turn_id.is_none());
        let raw = serde_json::to_value(&doc).expect("serialise");
        for key in [
            "decision_id",
            "decision_title",
            "decision_status",
            "decision_supersedes",
            "law_id",
            "law_severity",
            "law_tier",
            "turn_id",
        ] {
            assert!(
                raw.get(key).is_none(),
                "Memory document must not carry `{key}`",
            );
        }
    }

    #[test]
    fn malformed_decision_payload_does_not_panic() {
        // Defensive: a worker should tolerate junk payloads rather
        // than poison the whole batch. The serde-from_value branch
        // returns Err and the projection silently skips.
        let doc = ready(build_doc(
            &evt(
                "dec-bad",
                Kind::Decision,
                json!({ "title": "no decision_id" }),
            ),
            false,
            1024 * 1024,
        ));
        assert!(doc.decision_id.is_none());
        assert!(doc.decision_title.is_none());
    }

    #[test]
    fn round_trip_preserves_top_level_fields_through_serde() {
        let mut e = evt(
            "dec-rt",
            Kind::Decision,
            json!({
                "decision_id": "ADR-RT",
                "title": "Round trip",
                "status": "accepted",
                "body": "body",
                "tags": []
            }),
        );
        // Force topics non-empty so the field round-trips — Document
        // serialises `topics` with `skip_serializing_if =
        // "Vec::is_empty"`, and the deserialiser does not default
        // missing arrays. Spec 08 documents this trade-off.
        e.classifier.topics = vec!["search".into()];
        let doc = ready(build_doc(&e, false, 1024 * 1024));
        let raw = serde_json::to_string(&doc).expect("serialise");
        let parsed: Document = serde_json::from_str(&raw).expect("round-trip");
        assert_eq!(parsed.decision_id.as_deref(), Some("ADR-RT"));
        assert_eq!(parsed.decision_title.as_deref(), Some("Round trip"));
        assert_eq!(parsed.decision_status.as_deref(), Some("accepted"));
    }
}
