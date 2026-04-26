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

use cortex_core::events::Kind;
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
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(tool_call_id.clone()));
    props.insert(
        "content_hash".to_string(),
        Value::String(event.content_hash.clone()),
    );
    patch.nodes.push(NodeOp {
        label: "ToolCall".to_string(),
        natural_key: tool_call_id.clone(),
        props,
    });
    // Without the per-kind payload struct we cannot reach the parent
    // `turn_id`; anchor the ToolCall under the Session for now and let
    // a richer mapping layer add the `HAS_TOOL_CALL(Turn → ToolCall)`
    // edge when the per-kind payload is available.
    patch.edges.push(EdgeOp {
        edge_type: "HAS_TOOL_CALL".to_string(),
        from_label: "Session".to_string(),
        from_key: session_id.to_string(),
        to_label: "ToolCall".to_string(),
        to_key: tool_call_id,
        props: BTreeMap::new(),
    });

    // Fold context.repo + context.path into an Artifact + IN_REPO when
    // the envelope carries them.
    if let (Some(repo), Some(path)) = (
        event.context_repo.as_deref(),
        event.context_path.as_deref(),
    ) {
        emit_artifact_node(repo, path, &event.content_hash, patch);
    }
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
    let decision_id = event.event_id.clone();
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(decision_id.clone()));
    patch.nodes.push(NodeOp {
        label: "Decision".to_string(),
        natural_key: decision_id,
        props,
    });
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
    let violation_id = event.event_id.clone();
    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(violation_id.clone()));
    patch.nodes.push(NodeOp {
        label: "LawViolation".to_string(),
        natural_key: violation_id,
        props,
    });
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
