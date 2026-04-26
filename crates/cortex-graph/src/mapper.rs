//! Event-to-graph mapper.
//!
//! Per spec 07 §Event-to-graph mapping the mapper turns a single
//! [`EnrichedEvent`] into a [`GraphPatch`] — a list of node and edge
//! upserts keyed by their natural identity (architecture §4.2). Examples
//! from the spec:
//!
//! - **`turn.*`** → upsert `Turn`, upsert `HAS_TURN(Session→Turn)`,
//!   create `Session` if missing.
//! - **`tool_call.*`** → upsert `ToolCall`, upsert
//!   `HAS_TOOL_CALL(Turn→ToolCall)`, upsert `TOUCHED(ToolCall→Artifact)`
//!   for each affected file, upsert `Artifact` and `IN_REPO`.
//! - **`decision.created`** → upsert `Decision`, add
//!   `LINKED_TO(Turn→Decision, role=proposed)`.
//! - **`law.violation`** → upsert `LawViolation`, upsert `OF(→Law)`,
//!   upsert `OBSERVED_IN(→Turn|ToolCall)`.
//!
//! This module currently produces the **identity-only** subset of those
//! patches: it always upserts the entity-level node implied by the event
//! kind plus a `Session` node and `HAS_TURN` / `HAS_*` linkage that can
//! be derived purely from the [`cortex_embedder::EnrichedEvent`] surface
//! without re-parsing the per-kind payload. The richer fields
//! (`TOUCHED` per file, `LINKED_TO`, `OF`, `OBSERVED_IN`) come online
//! when the mapper is wired to the per-kind payload structs from
//! `cortex-core`. Until then this is enough for `ensure_schema` smoke
//! tests and downstream worker plumbing — every emitted patch is real,
//! deterministic, and idempotent under replay.

use std::collections::BTreeMap;

use cortex_core::events::{
    DecisionPayload, Kind, LawViolationPayload, ToolCall as ToolCallPayload, TouchedArtifact,
};
use serde_json::{json, Value};

use crate::identity::artifact_natural_key;
use crate::patch::{EdgeOp, GraphPatch, NodeOp};
use crate::EnrichedEvent;

/// Map one enriched event into a graph patch.
///
/// The patch is always idempotent under replay because every node uses
/// its natural key (`event_id`, `session_id`, or
/// `repo|path|content_hash` for `Artifact`).
pub fn map_event_to_patch(event: &EnrichedEvent) -> GraphPatch {
    let mut patch = GraphPatch::empty();

    // Every event implies a Session node — without it we cannot anchor
    // the per-kind node into the graph. The full `Session` props
    // (`started_at`, `adapter`, `model`) live on the `session.start`
    // event; here we upsert with just the natural key, and Cypher
    // `MERGE` semantics will keep any earlier-set props intact.
    let session_id = session_id_of(event);
    let mut session_props = BTreeMap::new();
    session_props.insert("id".to_string(), Value::String(session_id.clone()));
    patch.nodes.push(NodeOp {
        label: "Session".to_string(),
        natural_key: session_id.clone(),
        props: session_props,
    });

    match event.kind {
        Kind::Turn => emit_turn(event, &session_id, &mut patch),
        Kind::ToolCall => emit_tool_call(event, &session_id, &mut patch),
        Kind::AgentCall => emit_tool_call(event, &session_id, &mut patch),
        Kind::Memory => emit_memory(event, &session_id, &mut patch),
        Kind::Decision => emit_decision(event, &mut patch),
        Kind::Analysis => emit_analysis(event, &mut patch),
        Kind::LawViolation => emit_law_violation(event, &mut patch),
        Kind::Artifact => emit_artifact(event, &mut patch),
    }

    patch
}

fn emit_turn(event: &EnrichedEvent, session_id: &str, patch: &mut GraphPatch) {
    let turn_id = event.event_id.clone();
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(turn_id.clone()));
    props.insert("session_id".to_string(), Value::String(session_id.to_string()));
    props.insert(
        "content_hash".to_string(),
        Value::String(event.content_hash.clone()),
    );
    patch.nodes.push(NodeOp {
        label: "Turn".to_string(),
        natural_key: turn_id.clone(),
        props,
    });
    patch.edges.push(EdgeOp {
        edge_type: "HAS_TURN".to_string(),
        from_label: "Session".to_string(),
        from_key: session_id.to_string(),
        to_label: "Turn".to_string(),
        to_key: turn_id,
        props: BTreeMap::new(),
    });
}

fn emit_tool_call(event: &EnrichedEvent, session_id: &str, patch: &mut GraphPatch) {
    let tool_call_id = event.event_id.clone();
    let payload: Option<ToolCallPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(tool_call_id.clone()));
    props.insert(
        "content_hash".to_string(),
        Value::String(event.content_hash.clone()),
    );
    if let Some(p) = payload.as_ref() {
        props.insert(
            "tool_name".to_string(),
            Value::String(p.tool_name.clone()),
        );
        props.insert("outcome".to_string(), Value::String(p.outcome.clone()));
        if let Some(d) = p.duration_ms {
            props.insert("duration_ms".to_string(), Value::from(d));
        }
    }
    patch.nodes.push(NodeOp {
        label: "ToolCall".to_string(),
        natural_key: tool_call_id.clone(),
        props,
    });

    // HAS_TOOL_CALL anchoring: prefer the parent Turn when the envelope
    // carries `parent_event_id` (the spec-correct shape: `Turn →
    // ToolCall`); otherwise fall back to the Session anchor — keeps the
    // graph connected even when an out-of-order `tool_call` arrives
    // before its `turn.start`.
    if let Some(parent) = event.parent_event_id.as_deref() {
        patch.edges.push(EdgeOp {
            edge_type: "HAS_TOOL_CALL".to_string(),
            from_label: "Turn".to_string(),
            from_key: parent.to_string(),
            to_label: "ToolCall".to_string(),
            to_key: tool_call_id.clone(),
            props: BTreeMap::new(),
        });
    } else {
        patch.edges.push(EdgeOp {
            edge_type: "HAS_TOOL_CALL".to_string(),
            from_label: "Session".to_string(),
            from_key: session_id.to_string(),
            to_label: "ToolCall".to_string(),
            to_key: tool_call_id.clone(),
            props: BTreeMap::new(),
        });
    }

    // Fold context.repo + context.path into an Artifact + IN_REPO when
    // the envelope carries them.
    if let (Some(repo), Some(path)) = (
        event.context_repo.as_deref(),
        event.context_path.as_deref(),
    ) {
        emit_artifact_node(repo, path, &event.content_hash, patch);
    }

    // Per spec 07 §Event-to-graph mapping: every `TouchedArtifact` on a
    // `tool_call.*` payload becomes a TOUCHED edge plus the matching
    // Artifact + IN_REPO upserts. The Artifact's `content_hash` is not
    // carried by `TouchedArtifact` itself (schema gap — TouchedArtifact
    // only has `kind` + `path`); we reuse the ToolCall's own
    // `content_hash` as a stable proxy so re-runs of the same ToolCall
    // collapse onto the same Artifact key. When the schema gains a
    // per-touched content_hash (future spec change) the only line that
    // moves is the third arg here.
    if let (Some(repo), Some(payload)) = (event.context_repo.as_deref(), payload.as_ref()) {
        for touched in &payload.touched {
            emit_touched_edge(repo, touched, &event.content_hash, &tool_call_id, patch);
        }
    }
}

fn emit_touched_edge(
    repo: &str,
    touched: &TouchedArtifact,
    content_hash: &str,
    tool_call_id: &str,
    patch: &mut GraphPatch,
) {
    emit_artifact_node(repo, &touched.path, content_hash, patch);
    let artifact_key = artifact_natural_key(repo, &touched.path, content_hash);
    let mut edge_props = BTreeMap::new();
    edge_props.insert(
        "operation".to_string(),
        Value::String(touched.kind.clone()),
    );
    patch.edges.push(EdgeOp {
        edge_type: "TOUCHED".to_string(),
        from_label: "ToolCall".to_string(),
        from_key: tool_call_id.to_string(),
        to_label: "Artifact".to_string(),
        to_key: artifact_key,
        props: edge_props,
    });
}

fn emit_artifact(event: &EnrichedEvent, patch: &mut GraphPatch) {
    if let (Some(repo), Some(path)) = (
        event.context_repo.as_deref(),
        event.context_path.as_deref(),
    ) {
        emit_artifact_node(repo, path, &event.content_hash, patch);
    }
}

fn emit_artifact_node(repo: &str, path: &str, content_hash: &str, patch: &mut GraphPatch) {
    let key = artifact_natural_key(repo, path, content_hash);
    let mut props = BTreeMap::new();
    props.insert("natural_key".to_string(), Value::String(key.clone()));
    props.insert("repo".to_string(), Value::String(repo.to_string()));
    props.insert("path".to_string(), Value::String(path.to_string()));
    props.insert(
        "content_hash".to_string(),
        Value::String(content_hash.to_string()),
    );
    patch.nodes.push(NodeOp {
        label: "Artifact".to_string(),
        natural_key: key.clone(),
        props,
    });

    let mut repo_props = BTreeMap::new();
    repo_props.insert("repo".to_string(), Value::String(repo.to_string()));
    patch.nodes.push(NodeOp {
        label: "Repo".to_string(),
        natural_key: repo.to_string(),
        props: repo_props,
    });

    patch.edges.push(EdgeOp {
        edge_type: "IN_REPO".to_string(),
        from_label: "Artifact".to_string(),
        from_key: key,
        to_label: "Repo".to_string(),
        to_key: repo.to_string(),
        props: BTreeMap::new(),
    });
}

fn emit_memory(event: &EnrichedEvent, session_id: &str, patch: &mut GraphPatch) {
    let memory_id = event.event_id.clone();
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(memory_id.clone()));
    patch.nodes.push(NodeOp {
        label: "Memory".to_string(),
        natural_key: memory_id.clone(),
        props,
    });
    patch.edges.push(EdgeOp {
        edge_type: "REMEMBERS".to_string(),
        from_label: "Session".to_string(),
        from_key: session_id.to_string(),
        to_label: "Memory".to_string(),
        to_key: memory_id,
        props: BTreeMap::new(),
    });
}

fn emit_decision(event: &EnrichedEvent, patch: &mut GraphPatch) {
    let payload: Option<DecisionPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();

    // Decision natural key is the payload's `decision_id` (architecture
    // §4.2 — ULIDs are minted upstream, never here). Falls back to
    // `event_id` when the payload has been redacted away so the patch
    // stays connected.
    let decision_key = payload
        .as_ref()
        .map(|p| p.decision_id.clone())
        .unwrap_or_else(|| event.event_id.clone());

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(decision_key.clone()));
    if let Some(p) = payload.as_ref() {
        props.insert("title".to_string(), Value::String(p.title.clone()));
        props.insert("status".to_string(), Value::String(p.status.clone()));
        if !p.tags.is_empty() {
            props.insert(
                "tags".to_string(),
                Value::Array(p.tags.iter().cloned().map(Value::String).collect()),
            );
        }
    }
    patch.nodes.push(NodeOp {
        label: "Decision".to_string(),
        natural_key: decision_key.clone(),
        props,
    });

    // LINKED_TO(Turn → Decision, role=<status>) — the parent Turn is the
    // anchor for the Decision in the graph; the role prop carries the
    // decision lifecycle stage straight from the payload.
    if let Some(parent) = event.parent_event_id.as_deref() {
        let role = payload
            .as_ref()
            .map(|p| p.status.clone())
            .unwrap_or_else(|| "proposed".to_string());
        let mut edge_props = BTreeMap::new();
        edge_props.insert("role".to_string(), Value::String(role));
        patch.edges.push(EdgeOp {
            edge_type: "LINKED_TO".to_string(),
            from_label: "Turn".to_string(),
            from_key: parent.to_string(),
            to_label: "Decision".to_string(),
            to_key: decision_key.clone(),
            props: edge_props,
        });
    }

    // SUPERSEDES(Decision → Decision) when the payload names a prior
    // decision. The superseded decision's full props get filled in by
    // its own emit pass — we MERGE on id-only here to keep the graph
    // connected even if the prior decision lands later.
    if let Some(p) = payload.as_ref() {
        if let Some(prior_id) = p.supersedes.as_deref() {
            patch.edges.push(EdgeOp {
                edge_type: "SUPERSEDES".to_string(),
                from_label: "Decision".to_string(),
                from_key: decision_key,
                to_label: "Decision".to_string(),
                to_key: prior_id.to_string(),
                props: BTreeMap::new(),
            });
        }
    }
}

fn emit_analysis(event: &EnrichedEvent, patch: &mut GraphPatch) {
    let analysis_id = event.event_id.clone();
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(analysis_id.clone()));
    patch.nodes.push(NodeOp {
        label: "Analysis".to_string(),
        natural_key: analysis_id,
        props,
    });
}

fn emit_law_violation(event: &EnrichedEvent, patch: &mut GraphPatch) {
    let payload: Option<LawViolationPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();

    // Architecture §4.2 — `LawViolation.id` is a ULID minted upstream;
    // the payload carries it as `violation_id`. Fall back to `event_id`
    // when the payload has been redacted away.
    let violation_key = payload
        .as_ref()
        .map(|p| p.violation_id.clone())
        .unwrap_or_else(|| event.event_id.clone());

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(violation_key.clone()));
    if let Some(p) = payload.as_ref() {
        props.insert("law_id".to_string(), Value::String(p.law_id.clone()));
        props.insert(
            "severity".to_string(),
            Value::String(p.severity.clone()),
        );
        props.insert("message".to_string(), Value::String(p.message.clone()));
        if let Some(tier) = p.tier {
            props.insert("tier".to_string(), Value::from(tier));
        }
    }
    patch.nodes.push(NodeOp {
        label: "LawViolation".to_string(),
        natural_key: violation_key.clone(),
        props,
    });

    // OF(LawViolation → Law) anchors the violation under its Law.
    // Spec 13 will own full Law-node provenance (title, severity,
    // version); until then we MERGE the Law on its `id` only — `SET +=`
    // keeps any richer props that spec 13 lands without overwriting.
    if let Some(p) = payload.as_ref() {
        let law_id = p.law_id.clone();
        let mut law_props = BTreeMap::new();
        law_props.insert("id".to_string(), Value::String(law_id.clone()));
        patch.nodes.push(NodeOp {
            label: "Law".to_string(),
            natural_key: law_id.clone(),
            props: law_props,
        });
        patch.edges.push(EdgeOp {
            edge_type: "OF".to_string(),
            from_label: "LawViolation".to_string(),
            from_key: violation_key,
            to_label: "Law".to_string(),
            to_key: law_id,
            props: BTreeMap::new(),
        });
    }

    // OBSERVED_IN(LawViolation → Turn|ToolCall) deferred — schema gap:
    // `LawViolationPayload.observed_event_id` is just an id with no
    // kind discriminator, so the writer cannot pick the right target
    // label without risking phantom Turn / ToolCall nodes via MERGE.
    // Tracked under graph-writer task §4.5; lands once the payload
    // gains an `observed_event_kind` field.
}

/// Best-effort session id derivation. The
/// [`cortex_embedder::EnrichedEvent`] type does not surface the
/// envelope's `session_id` directly today, so we read it from the
/// classifier `extras` bag if the upstream put it there, falling back
/// to the event id so the graph stays connected even when the hint is
/// missing. Any improvement to `EnrichedEvent` flows in here without
/// changing the public mapper API.
fn session_id_of(event: &EnrichedEvent) -> String {
    if let Some(Value::String(s)) = lookup_extras(event, "session_id") {
        if !s.is_empty() {
            return s.clone();
        }
    }
    event.event_id.clone()
}

fn lookup_extras<'a>(event: &'a EnrichedEvent, key: &str) -> Option<&'a Value> {
    // The redacted payload itself is the most reliable place to find
    // session_id at this layer of the stack — every per-kind payload
    // either carries it directly or inherits it from the envelope.
    let _ = json!(null); // anchor `serde_json` import for future expansion
    event.redacted_payload.get(key)
}
