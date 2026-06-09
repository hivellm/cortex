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

use crate::classifier::{ClassifierOutput, PiiRisk, Severity};
use cortex_core::events::{
    AgentCall, AnalysisPayload, ArtifactPayload, DecisionPayload, Kind, KnowledgePayload,
    LawPayload, LawViolationPayload, LearningPayload, MemoryPayload, ToolCall, Turn,
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
    // Minimal fallback: kind label + event_id so genuinely empty payloads
    // still produce an indexable document rather than being silently dropped.
    let minimal = format!("{} {}", kind_label(event.kind), event.event_id);
    let chosen = select_body(&raw, summary, max_body_bytes, &minimal);
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
        // Phase20 §5.2 — project the envelope's occurred_at (epoch ms)
        // plumbed through the classifier worker. Was previously hard-
        // coded 0, which left the per-repo `ts` sortable axis useless
        // and made `consolidation_costs` return zero buckets for every
        // time window.
        ts: event.occurred_at_ms,
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
        project_id: None,
        branch_id: None,
        lifecycle: None,
        valid_from_unix: None,
        valid_to_unix: None,
        superseded_at_unix: None,
        ext: BTreeMap::new(),
    };

    apply_extensions(event, &mut doc);
    apply_top_level_projection(event, &mut doc);
    apply_bitemporal_projection(event, &mut doc);
    BuildOutcome::Ready(Box::new(doc))
}

fn compute_doc_id(event: &EnrichedEvent, bootstrap: bool) -> String {
    // Phase22 P1 — content-addressable governance kinds (Decision,
    // LawViolation, Knowledge, Learning) are file-backed
    // (`.rulebook/{decisions,knowledge,learnings}`, `.claude/rules`)
    // and never-deleted (ADR-022 retention). Re-emitting the same file
    // must UPSERT, not duplicate. The live path keyed on the per-event
    // ULID, so ADR-023 landed in `cortex_decisions` 5x — identical
    // `content_hash`, distinct event ULIDs — burning recall slots
    // (`cortex_decision_search` returned the same ADR five times). Key
    // these kinds on the stable `(repo, path, content_hash)` tuple —
    // the SAME id space the bootstrap path uses — so live re-emits and
    // re-bootstraps both collapse to one Meili doc. Event-stream kinds
    // (Turn/ToolCall/AgentCall/Artifact) and prune-by-event_id kinds
    // (Memory via `cortex_forget`, Consolidation/TopicCard via the
    // retention pruner) keep the ULID id so those delete paths still
    // match on the primary key.
    let content_addressable = matches!(
        event.kind,
        Kind::Decision | Kind::LawViolation | Kind::Knowledge | Kind::Learning
    );
    if bootstrap || content_addressable {
        // The stable path needs `(repo, path, content_hash)`. Fall
        // back to the live id when the envelope can't supply both —
        // the bootstrap CLI + governance writers populate them, but
        // defensive handling keeps replays from a partially-populated
        // upstream usable.
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
        Kind::Law => {
            match serde_json::from_value::<LawPayload>(event.redacted_payload.clone()) {
                Ok(lp) => {
                    let mut out = lp.body.clone();
                    if let Some(title) = lp.title.as_deref() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(title);
                    }
                    (out, Some(lp.law_id))
                }
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
        // phase11r §3.1 — TopicCards index synthesis_markdown plus
        // any open_questions so the keyword lane matches against
        // both the prose body and the surfaced questions.
        Kind::TopicCard => {
            match serde_json::from_value::<cortex_core::events::TopicCardPayload>(
                event.redacted_payload.clone(),
            ) {
                Ok(t) => {
                    let mut body = t.synthesis_markdown;
                    if !t.open_questions.is_empty() {
                        body.push_str("\n\n");
                        for q in &t.open_questions {
                            body.push_str("- ");
                            body.push_str(q);
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
        Kind::Law => {
            serde_json::from_value::<LawPayload>(event.redacted_payload.clone())
                .map(|lp| {
                    lp.title
                        .as_deref()
                        .map(|t| format!("{}: {t}", lp.law_id))
                        .unwrap_or_else(|| lp.law_id.clone())
                })
                .unwrap_or_else(|_| event.event_id.clone())
        }
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
        Kind::TopicCard => serde_json::from_value::<cortex_core::events::TopicCardPayload>(
            event.redacted_payload.clone(),
        )
        .map(|t| t.topic_slug)
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
        Kind::Law => {
            if let Ok(lp) = serde_json::from_value::<LawPayload>(event.redacted_payload.clone()) {
                let mut payload = json!({ "law_id": lp.law_id });
                if let Some(t) = lp.title {
                    payload["title"] = Value::String(t);
                }
                if let Some(s) = lp.severity {
                    payload["severity"] = Value::String(s);
                }
                if let Some(d) = lp.detector {
                    payload["detector"] = Value::String(d);
                }
                doc.ext.insert("law".to_string(), payload);
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
        Kind::TopicCard => {
            if let Ok(t) = serde_json::from_value::<cortex_core::events::TopicCardPayload>(
                event.redacted_payload.clone(),
            ) {
                let synthesis_age_d = synthesis_age_days(&t.last_rev_at);
                doc.ext.insert(
                    "topic_card".to_string(),
                    json!({
                        "topic_card_id": t.topic_card_id,
                        "topic_slug": t.topic_slug,
                        "revision": t.revision,
                        "confidence": t.confidence,
                        "synthesis_age_d": synthesis_age_d,
                        "contradictions_count": t.contradictions.len(),
                        "repos": t.repos,
                        "synthesis_model": t.synthesis_model,
                        "events_since_last_rev": t.events_since_last_rev,
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
            } else {
                // Phase20 §7 — rulebook-shaped law payloads
                // (`.claude/rules/*.md` + the rulebook v5.3.0
                // generator) don't match the canonical
                // LawViolationPayload schema; they ship a plain
                // `{body, detector, law_id, severity, title}`
                // object instead. Without this fallback the
                // strict deserialize fails silently and the
                // governance index never gets `law_id` /
                // `law_severity` stamped, so
                // `cortex_law_violations?law_id=...` returns
                // empty for the entire corpus. Probe the raw
                // payload for the same keys when the strict path
                // fails so the filter axis works.
                if let Some(id) = event.redacted_payload.get("law_id").and_then(Value::as_str) {
                    doc.law_id = Some(id.to_string());
                }
                if let Some(sev) = event
                    .redacted_payload
                    .get("severity")
                    .and_then(Value::as_str)
                {
                    doc.law_severity = Some(sev.to_string());
                }
                // `tier` is rarely set on rulebook-shaped payloads;
                // leave it as None to avoid misclassification.
            }
        }
        Kind::Law => {
            if let Ok(lp) = serde_json::from_value::<LawPayload>(event.redacted_payload.clone()) {
                doc.law_id = Some(lp.law_id);
                doc.law_severity = lp.severity;
            } else {
                if let Some(id) = event.redacted_payload.get("law_id").and_then(Value::as_str) {
                    doc.law_id = Some(id.to_string());
                }
                if let Some(sev) = event.redacted_payload.get("severity").and_then(Value::as_str) {
                    doc.law_severity = Some(sev.to_string());
                }
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
        | Kind::Consolidation
        | Kind::TopicCard => {
            // No top-level projection for these kinds — their overlay
            // contracts (if any) read from `ext.<family>.*` already.
        }
    }
}

/// Phase18 §2.6 — project the bitemporal scoping columns onto the
/// Meili doc. Mirrors the graph-writer stamp in
/// `crates/cortex-workers/src/graph/bitemporal.rs` so the temporal
/// classifier (§3) can filter / sort Meili hits on the same axes
/// the graph walk uses. The `*_unix` columns are second-precision
/// epoch ints (per ADR-018) so Meili sortable range ops stay cheap.
///
/// `valid_from_unix` is always populated from the envelope's
/// `occurred_at_ms` (phase20 §5.2). `valid_to_unix` and
/// `superseded_at_unix` default to absent — the SUPERSEDES /
/// OBSOLETES edges land them later via the consolidator-side stamp
/// (§3.9 audit channel). `project_id` derives from
/// `event.context_repo` (lower-cased; falls back to `"unknown"`
/// matching the graph writer). `branch_id` is `"main"` until §2.12
/// ships per-project branches. `lifecycle` is `"active"` by default;
/// `apply_top_level_projection` may have already stamped a
/// payload-derived override (e.g. `DecisionPayload.status =
/// superseded`) — preserve it.
fn apply_bitemporal_projection(event: &EnrichedEvent, doc: &mut Document) {
    if doc.project_id.is_none() {
        let project = event
            .context_repo
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string());
        doc.project_id = Some(project);
    }
    if doc.branch_id.is_none() {
        doc.branch_id = Some("main".to_string());
    }
    if doc.lifecycle.is_none() {
        // `decision_status` is the only payload-derived lifecycle
        // signal projected at the top level today; map it onto the
        // shared `lifecycle` axis so a single classifier rule
        // applies across kinds. Anything else defaults to `active`.
        let lifecycle = doc
            .decision_status
            .clone()
            .unwrap_or_else(|| "active".to_string());
        doc.lifecycle = Some(lifecycle);
    }
    if doc.valid_from_unix.is_none() && event.occurred_at_ms > 0 {
        doc.valid_from_unix = Some(event.occurred_at_ms / 1_000);
    }
    // `valid_to_unix` and `superseded_at_unix` stay absent — the
    // bitemporal contract treats NULL as "still valid" / "still
    // believed" per ADR-018 §1.1. The SUPERSEDES backfill in §2.11
    // stamps them on the target row of each supersession event.
}

/// Days between `last_rev_at` (RFC 3339) and now (UTC). Returns `0`
/// when the timestamp is unparseable or in the future — the
/// pre-thinking renderer reads this value to drive the staleness
/// advisory, and surfacing a negative / huge nonsense value would
/// trip the `> stale-topic-card` line for every malformed input.
fn synthesis_age_days(last_rev_at: &str) -> u32 {
    let parsed = match chrono::DateTime::parse_from_rfc3339(last_rev_at) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return 0,
    };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(parsed);
    let days = delta.num_days();
    if days < 0 {
        0
    } else {
        days as u32
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
        Kind::Law => "law",
        Kind::LawViolation => "law_violation",
        Kind::Artifact => "artifact",
        Kind::Knowledge => "knowledge",
        Kind::Learning => "learning",
        Kind::Consolidation => "consolidation",
        Kind::TopicCard => "topic_card",
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
    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
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
            occurred_at_ms: 0,
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

    fn evt_with_source(
        event_id: &str,
        kind: Kind,
        content_hash: &str,
        repo: &str,
        path: &str,
        payload: serde_json::Value,
    ) -> EnrichedEvent {
        EnrichedEvent {
            event_id: event_id.to_string(),
            kind,
            content_hash: content_hash.to_string(),
            redacted_payload: payload,
            classifier: classifier(event_id),
            context_repo: Some(repo.to_string()),
            context_path: Some(path.to_string()),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
        }
    }

    #[test]
    fn live_decision_reemit_collapses_to_one_stable_doc_id() {
        // Phase22 P1 — ADR-023 was ingested 5x via the live path
        // (identical content_hash, distinct event ULIDs) and
        // `cortex_decision_search` returned the same decision five
        // times. Two live emits of the same decision file MUST yield
        // the same Meili primary key so Meili upserts instead of
        // duplicating.
        let payload = json!({
            "decision_id": "ADR-0023",
            "title": "Supersession edges",
            "status": "proposed",
            "body": "SUPERSEDES / OBSOLETES / EVOLVES_FROM",
            "tags": []
        });
        let a = ready(build_doc(
            &evt_with_source(
                "01LIVEAAA",
                Kind::Decision,
                "hash-adr-023",
                "cortex",
                ".rulebook/decisions/023.md",
                payload.clone(),
            ),
            false,
            1024 * 1024,
        ));
        let b = ready(build_doc(
            &evt_with_source(
                "01LIVEBBB",
                Kind::Decision,
                "hash-adr-023",
                "cortex",
                ".rulebook/decisions/023.md",
                payload,
            ),
            false,
            1024 * 1024,
        ));
        assert_eq!(
            a.id, b.id,
            "live re-emit of the same decision must share a doc id"
        );
        assert!(
            a.id.starts_with("bootstrap:"),
            "content-addressable kinds use the stable id space, got {}",
            a.id
        );
    }

    #[test]
    fn live_turn_keeps_per_event_id_even_with_repo_path() {
        // Event-stream kinds must NOT collapse — two turns are distinct
        // events even when they share a repo/path. Keep the ULID id so
        // the retention pruner's delete-by-event_id still matches.
        let mk = |id: &str| {
            ready(build_doc(
                &evt_with_source(
                    id,
                    Kind::Turn,
                    "hash-same",
                    "cortex",
                    "src/lib.rs",
                    json!({ "user_message": "hi", "assistant_message": "yo" }),
                ),
                false,
                1024 * 1024,
            ))
        };
        let a = mk("01TURNAAA");
        let b = mk("01TURNBBB");
        assert_ne!(a.id, b.id, "turns must keep distinct per-event ids");
        assert_eq!(a.id, "01TURNAAA");
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
