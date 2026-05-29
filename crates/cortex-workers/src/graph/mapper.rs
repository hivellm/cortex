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
//! be derived purely from the [`crate::embedder::EnrichedEvent`] surface
//! without re-parsing the per-kind payload. The richer fields
//! (`TOUCHED` per file, `LINKED_TO`, `OF`, `OBSERVED_IN`) come online
//! when the mapper is wired to the per-kind payload structs from
//! `cortex-core`. Until then this is enough for `ensure_schema` smoke
//! tests and downstream worker plumbing — every emitted patch is real,
//! deterministic, and idempotent under replay.

use std::collections::BTreeMap;

use cortex_core::events::{
    AgentCall as AgentCallPayload, AnalysisPayload, DecisionPayload, EvidenceKind, Kind,
    LawViolationPayload, MemoryPayload, ToolCall as ToolCallPayload, TopicCardPayload,
    TouchedArtifact, Turn as TurnPayload,
};
use serde_json::{json, Value};

use crate::embedder::{ChunkSource, Chunker, CodeChunker};

use super::identity::{artifact_natural_key, symbol_natural_key};
use super::patch::{ConflictPolicy, EdgeOp, GraphPatch, NodeOp};
use crate::embedder::EnrichedEvent;

/// Cap a human-readable label so a single node never explodes the
/// graph viewer with multi-paragraph text. The cap is generous —
/// 96 chars — because Nexus clients (and our Cytoscape renderer)
/// already truncate display text further.
fn clip_display(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out = String::with_capacity(max + 1);
    for (i, ch) in trimmed.chars().enumerate() {
        if i >= max {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// Stamp every prop a graph viewer might use to render a node's
/// caption. Different Nexus / Cytoscape / Bloom builds prefer
/// different keys (`label` / `name` / `display` / `caption`), and
/// our own dashboard's `node_label()` helper falls through every
/// candidate before defaulting to the natural-key id. Setting all
/// of them at once costs ~80 bytes per node and means whichever
/// viewer the operator opens reads SOMETHING legible — instead of
/// the unrendered ULID we were leaking into the Nexus UI.
fn stamp_display_label(props: &mut BTreeMap<String, Value>, label: &str) {
    let clipped = clip_display(label, 96);
    let v = Value::String(clipped);
    props.insert("label".to_string(), v.clone());
    props.insert("display".to_string(), v.clone());
    props.insert("caption".to_string(), v.clone());
    props.insert("name".to_string(), v);
}

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
    // event; here we upsert with just the natural key plus a
    // best-effort display label, and Cypher `MERGE` semantics will
    // keep any earlier-set props intact.
    let session_id = session_id_of(event);
    let mut session_props = BTreeMap::new();
    session_props.insert("id".to_string(), Value::String(session_id.clone()));
    // Short version of the session id for human use in the Nexus
    // browser. Sessions are normally 26-char ULIDs so the first 12
    // chars uniquely identify a session in practice.
    let session_short = session_id.chars().take(12).collect::<String>();
    stamp_display_label(&mut session_props, &format!("Session {session_short}"));
    patch.nodes.push(NodeOp {
        label: "Session".to_string(),
        natural_key: session_id.clone(),
        external_id: Some(session_id.clone()),
        conflict_policy: ConflictPolicy::default(),
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
        // phase10e — knowledge / learnings ride alongside the
        // memory mapping for now: a single typed node keyed by
        // the event id, attached to the Session via the same
        // `OWNS` edge memories use. Dedicated `:Knowledge` /
        // `:Learning` labels surface in the canonical-label
        // table below so the dashboard's graph view colour-codes
        // them; richer relationship edges (`(:Knowledge)
        // -[:RELATES_TO]->(:Decision)`) ride the
        // classifier-entities path and require no per-kind
        // mapping here.
        Kind::Knowledge => emit_memory(event, &session_id, &mut patch),
        Kind::Learning => emit_memory(event, &session_id, &mut patch),
        // Phase11j — Consolidations ride the memory mapping path
        // (Session OWNS Consolidation), same as knowledge /
        // learnings. The dedicated `:Consolidation` label below
        // gives the dashboard a colour-coded slot.
        Kind::Consolidation => emit_memory(event, &session_id, &mut patch),
        // phase11r §3.2 — TopicCard gets a dedicated emitter that lays
        // down a `:TopicCard` node keyed by the deterministic
        // `topic_card_id`, an `:OWNS` link from the Session for graph
        // connectivity (matching the memory / consolidation pattern),
        // one `:EVIDENCE_OF` edge per evidence item with the target
        // label varying by `EvidenceKind`, and `:RELATED_TO` edges
        // (bidirectional, deduped) for every entry in
        // `related_topic_ids`. Heuristic sibling links — never block
        // anything, just light up the graph view + power the
        // `cortex_topic_neighbors` MCP walk that lands in §4.3.
        Kind::TopicCard => emit_topic_card(event, &session_id, &mut patch),
    }

    // Sonnet-extracted entity/relation layer. Independent from the
    // structural mapping above — the structural pass produces the
    // canonical Session→Turn→ToolCall→Artifact skeleton, this pass
    // adds the SEMANTIC layer (REFERENCES / IMPLEMENTS / DISCUSSES /
    // …) the user asked for so retrieval can traverse the meaning
    // dimension instead of relying on full-text hits alone.
    emit_classifier_entities(event, &mut patch);

    // phase18 §2.1 — stamp the bitemporal scoping columns
    // (`project_id`, `branch_id`, `valid_from`, `recorded_at`,
    // `lifecycle`) on every node the patch carries. Idempotent on
    // values already set by the per-kind emitter (Decision keeps
    // its payload-derived status; etc.). The temporal classifier
    // (phase18 §3, blocked on phase14c golden-set evidence) reads
    // these columns at retrieval time to drop / demote /
    // keep-as-VALID per ADRs 018–023.
    super::bitemporal::stamp_bitemporal_props_on_patch(event, &mut patch);

    patch
}

/// Translate the classifier's `entities` + `relations` into graph
/// nodes + edges. Each entity becomes a typed node keyed by
/// `(entity_type, identifier)` so the same `Decision DEC-0042`
/// referenced from many events collapses to a single Nexus node.
/// Each relation becomes an edge anchored at the current event
/// (its primary node id), with the relation label as the edge
/// type. Skipped silently when the classifier emitted nothing
/// (static fallback, or Sonnet returned an empty list).
fn emit_classifier_entities(event: &EnrichedEvent, patch: &mut GraphPatch) {
    let entities = &event.classifier.entities;
    let relations = &event.classifier.relations;
    if entities.is_empty() && relations.is_empty() {
        return;
    }

    // Index entities by identifier so the relation pass below can
    // look up `entity_type` and pick the right Nexus label without
    // re-parsing.
    let mut by_id: BTreeMap<&str, &crate::classifier::types::ExtractedEntity> = BTreeMap::new();
    for ent in entities {
        by_id.insert(ent.identifier.as_str(), ent);
        let label = entity_type_to_label(&ent.entity_type);
        let mut props = BTreeMap::new();
        props.insert("id".to_string(), Value::String(ent.identifier.clone()));
        // Display caption for the Nexus / Cytoscape viewer. Prefer
        // the Sonnet-supplied human label (e.g. a decision title)
        // when present; fall back to the identifier itself, which
        // is always the most diagnostic thing the operator can
        // read (e.g. `DEC-0042`, `crates/cortex-api/src/lib.rs`).
        let display = ent
            .label
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ent.identifier.clone());
        stamp_display_label(&mut props, &display);
        // Carry the typed kind so dashboard surfaces can colour-code
        // even when the canonical label collapses to `Concept` (the
        // catch-all for novel entity types Sonnet emits).
        props.insert(
            "entity_type".to_string(),
            Value::String(ent.entity_type.clone()),
        );
        patch.nodes.push(NodeOp {
            label: label.to_string(),
            natural_key: format!("{}|{}", ent.entity_type, ent.identifier),
            external_id: Some(format!("{}|{}", ent.entity_type, ent.identifier)),
            conflict_policy: ConflictPolicy::default(),
            props,
        });
    }

    // Anchor each relation at the current event's primary node.
    // The kind dictates which natural key to use.
    let event_anchor = anchor_natural_key(event);
    let event_label = anchor_label_for_kind(event.kind);

    for rel in relations {
        // Resolve the target entity from the same record. Skip the
        // relation when the target wasn't declared — keeps the graph
        // free of phantom edges Sonnet occasionally hallucinates.
        let target = match by_id.get(rel.to.as_str()) {
            Some(e) => *e,
            None => continue,
        };
        let target_label = entity_type_to_label(&target.entity_type);
        let target_key = format!("{}|{}", target.entity_type, target.identifier);

        // Normalise the relation label to UPPER_SNAKE so the Nexus
        // schema stays inspectable. Drop relations whose label
        // doesn't validate (the prompt enumerates the closed set
        // but defensive parsing catches drift).
        let rel_label = match normalise_relation_label(&rel.relation) {
            Some(s) => s,
            None => continue,
        };

        let mut props = BTreeMap::new();
        props.insert(
            "extracted_by".to_string(),
            Value::String(event.classifier.model.clone()),
        );
        props.insert(
            "event_id".to_string(),
            Value::String(event.event_id.clone()),
        );
        patch.edges.push(EdgeOp {
            edge_type: rel_label,
            from_label: event_label.to_string(),
            from_key: event_anchor.clone(),
            to_label: target_label.to_string(),
            to_key: target_key,
            props,
        });
    }
}

/// Map an `entity_type` from the classifier prompt's controlled
/// list to its Nexus node label. Unknown types collapse to
/// `Concept` so a novel value Sonnet emits still creates a node
/// rather than silently dropping it.
fn entity_type_to_label(entity_type: &str) -> &'static str {
    match entity_type {
        "decision" => "Decision",
        "law" => "Law",
        "analysis" => "Analysis",
        "artifact" => "Artifact",
        "repo" => "Repo",
        "topic" => "Topic",
        "tool" => "Tool",
        "person" => "Person",
        "session" => "Session",
        "turn" => "Turn",
        // `concept` and any unknown / novel type — the catch-all so
        // the graph layer doesn't silently drop entities just
        // because Sonnet invented a new category we haven't
        // codified yet. Worst case: a `Concept` node with the
        // explicit `entity_type` prop preserved for later cleanup.
        _ => "Concept",
    }
}

/// Pick the structural natural key the current event was upserted
/// under by the per-kind emitters above. Used as the `from` of
/// every classifier-extracted relation.
fn anchor_natural_key(event: &EnrichedEvent) -> String {
    match event.kind {
        Kind::Artifact => {
            // Artifacts use the spec-07 natural key
            // `repo|path|content_hash`. Reuse `artifact_natural_key`
            // so the value matches what `emit_artifact` upserted.
            let repo = event.context_repo.as_deref().unwrap_or("unknown");
            let path = event.context_path.as_deref().unwrap_or("unknown");
            artifact_natural_key(repo, path, &event.content_hash)
        }
        // Everything else uses event_id as its natural key.
        _ => event.event_id.clone(),
    }
}

fn anchor_label_for_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Turn => "Turn",
        Kind::ToolCall => "ToolCall",
        Kind::AgentCall => "AgentCall",
        Kind::Memory => "Memory",
        Kind::Decision => "Decision",
        Kind::Analysis => "Analysis",
        Kind::LawViolation => "LawViolation",
        Kind::Artifact => "Artifact",
        Kind::Knowledge => "Knowledge",
        Kind::Learning => "Learning",
        Kind::Consolidation => "Consolidation",
        Kind::TopicCard => "TopicCard",
    }
}

/// Validate + normalise a relation label against the closed set
/// the classifier prompt names. Returns `None` for labels outside
/// the vocabulary so we never write phantom edge types to Nexus.
fn normalise_relation_label(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase().replace([' ', '-'], "_");
    match upper.as_str() {
        "REFERENCES" | "IMPLEMENTS" | "FIXES" | "DISCUSSES" | "DEFINES" | "DEPENDS_ON"
        | "SUPERSEDES" | "OBSERVED_IN" | "TOUCHED" => Some(upper),
        _ => None,
    }
}

fn emit_turn(event: &EnrichedEvent, session_id: &str, patch: &mut GraphPatch) {
    let turn_id = event.event_id.clone();
    let payload: Option<TurnPayload> = serde_json::from_value(event.redacted_payload.clone()).ok();

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(turn_id.clone()));
    props.insert(
        "session_id".to_string(),
        Value::String(session_id.to_string()),
    );
    props.insert(
        "content_hash".to_string(),
        Value::String(event.content_hash.clone()),
    );
    // Display label — prefer the canonical `Turn.user_message`; fall
    // back to bootstrap's `turn.historical` payload shape
    // (`{role, message, evidence}`) so historical commits get a
    // useful name; finally fall back to the event id.
    let canonical_message = payload
        .as_ref()
        .map(|p| p.user_message.clone())
        .filter(|s| !s.is_empty());
    let bootstrap_message = canonical_message.clone().or_else(|| {
        event
            .redacted_payload
            .get("message")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    });
    let display = bootstrap_message
        .as_deref()
        .map(|s| clip_display(s, 96))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Turn {}", turn_id.chars().take(12).collect::<String>()));
    stamp_display_label(&mut props, &display);
    if let Some(msg) = bootstrap_message.as_deref() {
        if !msg.is_empty() {
            // Carry the full user message so downstream readers (the
            // dashboard's `node_label` in particular) can clip it on
            // their own; capped at 4 KB to keep individual props
            // small inside Nexus.
            props.insert(
                "user_message".to_string(),
                Value::String(clip_display(msg, 4096)),
            );
        }
    }
    if let Some(p) = payload.as_ref() {
        if let Some(reply) = p.assistant_message.as_deref() {
            if !reply.is_empty() {
                props.insert(
                    "assistant_message".to_string(),
                    Value::String(clip_display(reply, 4096)),
                );
            }
        }
    }
    patch.nodes.push(NodeOp {
        label: "Turn".to_string(),
        natural_key: turn_id.clone(),
        external_id: Some(turn_id.clone()),
        conflict_policy: ConflictPolicy::default(),
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

    // Best-effort display label — pulls the first touched path or
    // the input's `command` / `pattern` / `path` field so the
    // ToolCall reads as `Edit src/foo.rs` / `Bash cargo check` in
    // Nexus instead of a bare ULID.
    let display = match (event.kind, payload.as_ref()) {
        (Kind::AgentCall, _) => agent_call_display(event),
        (_, Some(p)) => format!("{} {}", p.tool_name, tool_call_target(p))
            .trim()
            .to_string(),
        (_, None) => format!(
            "ToolCall {}",
            tool_call_id.chars().take(12).collect::<String>()
        ),
    };
    let display = if display.is_empty() {
        format!(
            "ToolCall {}",
            tool_call_id.chars().take(12).collect::<String>()
        )
    } else {
        display
    };
    stamp_display_label(&mut props, &clip_display(&display, 96));

    if let Some(p) = payload.as_ref() {
        props.insert("tool_name".to_string(), Value::String(p.tool_name.clone()));
        props.insert("outcome".to_string(), Value::String(p.outcome.clone()));
        if let Some(d) = p.duration_ms {
            props.insert("duration_ms".to_string(), Value::from(d));
        }
        if let Some(target) = tool_call_target_field(p) {
            props.insert("target".to_string(), Value::String(target));
        }
    }
    // Stamp `kind` so downstream consumers (e.g. cortex-api's graph
    // endpoint) can distinguish a real ToolCall from an AgentCall
    // even though both share the `:ToolCall` label today.
    props.insert(
        "kind".to_string(),
        Value::String(
            match event.kind {
                Kind::AgentCall => "agent_call",
                _ => "tool_call",
            }
            .to_string(),
        ),
    );
    patch.nodes.push(NodeOp {
        label: "ToolCall".to_string(),
        natural_key: tool_call_id.clone(),
        external_id: Some(tool_call_id.clone()),
        conflict_policy: ConflictPolicy::default(),
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
    if let (Some(repo), Some(path)) = (event.context_repo.as_deref(), event.context_path.as_deref())
    {
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
    edge_props.insert("operation".to_string(), Value::String(touched.kind.clone()));
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
    if let (Some(repo), Some(path)) = (event.context_repo.as_deref(), event.context_path.as_deref())
    {
        emit_artifact_node(repo, path, &event.content_hash, patch);
        // Phase4c §2 — surface code symbols as first-class graph
        // nodes when the artifact is a recognised source file. The
        // CodeChunker is the same one cortex-embedder runs against
        // Vectorizer; reusing it keeps the symbol set in lockstep
        // with what the vector lane sees.
        emit_symbol_patches(event, repo, path, patch);
    }
}

/// Run the code chunker against an artifact event and emit one
/// `(:Symbol)-[:DEFINES]->(:Artifact)` pair per top-level
/// declaration the chunker recognises. Silently produces nothing
/// when the path's language is not in the chunker's grammar set,
/// when the payload has no body, or when no chunk surfaced a
/// symbol — phase4c §2.2 forbids logging an error in those cases
/// because most artifact events legitimately have no code symbols
/// to extract.
fn emit_symbol_patches(event: &EnrichedEvent, repo: &str, path: &str, patch: &mut GraphPatch) {
    let chunker = CodeChunker::new();
    let chunks = match chunker.chunk(event) {
        Ok(c) => c,
        Err(_) => return,
    };
    let artifact_key = artifact_natural_key(repo, path, &event.content_hash);
    let mut emitted: std::collections::BTreeSet<String> = Default::default();
    for chunk in chunks {
        // Spec §1.1 — `source == Code` filter. Sliding-window
        // fallback chunks carry a `Some("rust")` language label too,
        // but their `symbol` is None, so the empty-name check below
        // is the active gate; the source check is belt-and-braces.
        if chunk.metadata.source != ChunkSource::Code {
            continue;
        }
        let raw_name = match chunk.metadata.symbol.as_deref() {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => continue,
        };
        let language = chunk
            .metadata
            .language
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        // Spec §1.1 fallback — when the language carries no FQN
        // concept the chunker emits a bare name (`fn parse`).
        // Compose `<path>::<name>` so two same-named symbols in
        // different files don't collide on the natural key.
        let qualified_name = if raw_name.contains("::") || raw_name.contains('.') {
            raw_name.clone()
        } else {
            format!("{path}::{raw_name}")
        };
        let key = symbol_natural_key(repo, &language, &qualified_name);
        // Coalesce duplicates inside this single event — multiple
        // chunks can technically report the same FQN if a
        // declaration appears twice (e.g. an `impl` block with the
        // same name as the type). The coalescer handles this across
        // batches; we de-dupe within the same patch up front so the
        // emitted node count matches the unique-symbol count.
        if !emitted.insert(key.clone()) {
            continue;
        }
        let mut props = BTreeMap::new();
        props.insert("natural_key".to_string(), Value::String(key.clone()));
        props.insert("name".to_string(), Value::String(raw_name.clone()));
        props.insert("repo".to_string(), Value::String(repo.to_string()));
        props.insert("language".to_string(), Value::String(language));
        props.insert(
            "qualified_name".to_string(),
            Value::String(qualified_name.clone()),
        );
        // Display surface — the bare name reads better in Nexus
        // than the `repo|lang|qname` triple the natural key carries.
        stamp_display_label(&mut props, &raw_name);
        patch.nodes.push(NodeOp {
            label: "Symbol".to_string(),
            natural_key: key.clone(),
            external_id: Some(key.clone()),
            conflict_policy: ConflictPolicy::default(),
            props,
        });
        patch.edges.push(EdgeOp {
            edge_type: "DEFINES".to_string(),
            from_label: "Symbol".to_string(),
            from_key: key,
            to_label: "Artifact".to_string(),
            to_key: artifact_key.clone(),
            props: BTreeMap::new(),
        });
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
    // Display label = the path; that is the field a human scans.
    stamp_display_label(&mut props, &clip_display(path, 96));
    patch.nodes.push(NodeOp {
        label: "Artifact".to_string(),
        natural_key: key.clone(),
        external_id: Some(key.clone()),
        conflict_policy: ConflictPolicy::default(),
        props,
    });

    // Repo node MERGEs on `name` (per spec 07 schema constraint).
    // Stamp both `name` and a redundant `repo` field so old readers
    // that grep for `repo` keep working.
    let mut repo_props = BTreeMap::new();
    stamp_display_label(&mut repo_props, repo);
    repo_props.insert("repo".to_string(), Value::String(repo.to_string()));
    patch.nodes.push(NodeOp {
        label: "Repo".to_string(),
        natural_key: repo.to_string(),
        external_id: Some(repo.to_string()),
        conflict_policy: ConflictPolicy::default(),
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
    let payload: Option<MemoryPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(memory_id.clone()));
    let display = payload
        .as_ref()
        .map(|p| {
            let head = if p.name.is_empty() {
                p.description.as_deref().unwrap_or("Memory").to_string()
            } else {
                p.name.clone()
            };
            format!("{head} ({})", p.memory_type)
        })
        .unwrap_or_else(|| format!("Memory {}", memory_id.chars().take(12).collect::<String>()));
    stamp_display_label(&mut props, &clip_display(&display, 96));
    if let Some(p) = payload.as_ref() {
        props.insert("title".to_string(), Value::String(p.name.clone()));
        props.insert(
            "memory_type".to_string(),
            Value::String(p.memory_type.clone()),
        );
        props.insert("op".to_string(), Value::String(p.op.clone()));
    }
    patch.nodes.push(NodeOp {
        label: "Memory".to_string(),
        natural_key: memory_id.clone(),
        external_id: Some(memory_id.clone()),
        conflict_policy: ConflictPolicy::default(),
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

/// phase11r §3.2 — translate a `topic_card.*` envelope into:
///
///   `(:Session)-[:OWNS]->(:TopicCard)`
///   `(:TopicCard)-[:EVIDENCE_OF]->(:Decision|:Law|:Consolidation|:Turn)`
///   `(:TopicCard)-[:RELATED_TO]->(:TopicCard)` (both directions)
///
/// Natural key for the `:TopicCard` node is the deterministic
/// `topic_card_id` — re-emitting the same card across revisions
/// upserts onto the same node and bumps `revision` in-place.
fn emit_topic_card(event: &EnrichedEvent, session_id: &str, patch: &mut GraphPatch) {
    let payload: Option<TopicCardPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();
    let topic_card_key = payload
        .as_ref()
        .map(|p| p.topic_card_id.clone())
        .unwrap_or_else(|| event.event_id.clone());

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(topic_card_key.clone()));
    let display = payload
        .as_ref()
        .filter(|p| !p.topic_slug.is_empty())
        .map(|p| format!("{} (rev {})", p.topic_slug, p.revision))
        .unwrap_or_else(|| {
            format!(
                "TopicCard {}",
                topic_card_key.chars().take(12).collect::<String>()
            )
        });
    stamp_display_label(&mut props, &clip_display(&display, 96));
    if let Some(p) = payload.as_ref() {
        props.insert(
            "topic_slug".to_string(),
            Value::String(p.topic_slug.clone()),
        );
        props.insert("revision".to_string(), Value::from(p.revision));
        props.insert("confidence".to_string(), Value::from(p.confidence));
        props.insert(
            "synthesis_model".to_string(),
            Value::String(p.synthesis_model.clone()),
        );
        props.insert(
            "events_since_last_rev".to_string(),
            Value::from(p.events_since_last_rev),
        );
        props.insert(
            "last_rev_at".to_string(),
            Value::String(p.last_rev_at.clone()),
        );
        if !p.repos.is_empty() {
            props.insert(
                "repos".to_string(),
                Value::Array(p.repos.iter().cloned().map(Value::String).collect()),
            );
        }
    }
    patch.nodes.push(NodeOp {
        label: "TopicCard".to_string(),
        natural_key: topic_card_key.clone(),
        external_id: Some(topic_card_key.clone()),
        conflict_policy: ConflictPolicy::default(),
        props,
    });

    // Session anchor — keeps the topic card reachable from the
    // session graph the same way memories / consolidations are.
    patch.edges.push(EdgeOp {
        edge_type: "OWNS".to_string(),
        from_label: "Session".to_string(),
        from_key: session_id.to_string(),
        to_label: "TopicCard".to_string(),
        to_key: topic_card_key.clone(),
        props: BTreeMap::new(),
    });

    let payload = match payload {
        Some(p) => p,
        None => return,
    };

    // EVIDENCE_OF edges — target node label depends on the
    // `EvidenceKind` discriminator. The target node is created
    // by its own emitter when the source envelope is processed;
    // here we MERGE on (label, natural_key) so the edge stays
    // connected even when the evidence event lands later.
    let mut emitted_evidence: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    for ev in &payload.evidence {
        let target_label = match ev.kind {
            EvidenceKind::Consolidation => "Consolidation",
            EvidenceKind::Decision => "Decision",
            EvidenceKind::Law => "Law",
            EvidenceKind::Turn => "Turn",
        };
        if !emitted_evidence.insert((target_label.to_string(), ev.id.clone())) {
            continue;
        }
        let mut edge_props = BTreeMap::new();
        edge_props.insert(
            "evidence_kind".to_string(),
            Value::String(target_label.to_ascii_lowercase()),
        );
        edge_props.insert("cited_at_rev".to_string(), Value::from(ev.cited_at_rev));
        if let Some(w) = ev.weight {
            edge_props.insert("weight".to_string(), Value::from(w));
        }
        patch.edges.push(EdgeOp {
            edge_type: "EVIDENCE_OF".to_string(),
            from_label: "TopicCard".to_string(),
            from_key: topic_card_key.clone(),
            to_label: target_label.to_string(),
            to_key: ev.id.clone(),
            props: edge_props,
        });
    }

    // RELATED_TO edges — bidirectional + dedup. Self-references
    // (a card listing itself in `related_topic_ids`) are dropped.
    let mut emitted_related: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    for related_id in &payload.related_topic_ids {
        if related_id == &topic_card_key {
            continue;
        }
        for (from, to) in [
            (topic_card_key.clone(), related_id.clone()),
            (related_id.clone(), topic_card_key.clone()),
        ] {
            if !emitted_related.insert((from.clone(), to.clone())) {
                continue;
            }
            patch.edges.push(EdgeOp {
                edge_type: "RELATED_TO".to_string(),
                from_label: "TopicCard".to_string(),
                from_key: from,
                to_label: "TopicCard".to_string(),
                to_key: to,
                props: BTreeMap::new(),
            });
        }
    }
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
    let display = payload
        .as_ref()
        .filter(|p| !p.title.is_empty())
        .map(|p| format!("{} · {}", decision_key, p.title))
        .unwrap_or_else(|| format!("Decision {decision_key}"));
    stamp_display_label(&mut props, &clip_display(&display, 96));
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
        external_id: Some(decision_key.clone()),
        conflict_policy: ConflictPolicy::default(),
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
    // Two payload shapes feed this emitter:
    //
    //   1. Deep-analysis (spec 15) — `AnalysisPayload {
    //      analysis_id, question, status, decision_id?, panel }`.
    //   2. Bootstrap-imported audit/report (phase4e) — `{ title,
    //      status, body, source_path }`. No analysis_id; the
    //      event_id is the natural key.
    //
    // We try the canonical shape first; when it doesn't fit (or
    // produces an empty analysis_id), we fall back to the imported
    // shape. Either way the result is a labelled `Analysis` node
    // wired to its owning Repo via `ANALYZES`.
    let canonical: Option<AnalysisPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();
    let canonical_id = canonical
        .as_ref()
        .map(|p| p.analysis_id.clone())
        .filter(|s| !s.is_empty());

    let analysis_key = canonical_id.unwrap_or_else(|| event.event_id.clone());
    let imported_title = event
        .redacted_payload
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let imported_status = event
        .redacted_payload
        .get("status")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let imported_source_path = event
        .redacted_payload
        .get("source_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut props = BTreeMap::new();
    props.insert("id".to_string(), Value::String(analysis_key.clone()));
    let display = match canonical.as_ref() {
        Some(p) if !p.question.is_empty() => format!("{} · {}", analysis_key, p.question),
        _ => match imported_title.as_deref() {
            Some(t) => t.to_string(),
            None => format!("Analysis {analysis_key}"),
        },
    };
    stamp_display_label(&mut props, &clip_display(&display, 96));
    if let Some(p) = canonical.as_ref() {
        if !p.question.is_empty() {
            props.insert("title".to_string(), Value::String(p.question.clone()));
        }
        if !p.status.is_empty() {
            props.insert("status".to_string(), Value::String(p.status.clone()));
        }
        if let Some(decision) = p.decision_id.as_deref() {
            props.insert(
                "decision_id".to_string(),
                Value::String(decision.to_string()),
            );
        }
        if !p.panel.is_empty() {
            props.insert(
                "panel".to_string(),
                Value::Array(p.panel.iter().cloned().map(Value::String).collect()),
            );
        }
    }
    // Imported-shape fields are only set when they aren't already present
    // — canonical wins when both are populated (defensive against an
    // event that happens to carry both shapes).
    if let Some(t) = imported_title.as_deref() {
        props
            .entry("title".to_string())
            .or_insert_with(|| Value::String(t.to_string()));
    }
    if let Some(s) = imported_status.as_deref() {
        props
            .entry("status".to_string())
            .or_insert_with(|| Value::String(s.to_string()));
    }
    if let Some(p) = imported_source_path.as_deref() {
        props.insert("source_path".to_string(), Value::String(p.to_string()));
    }

    patch.nodes.push(NodeOp {
        label: "Analysis".to_string(),
        natural_key: analysis_key.clone(),
        external_id: Some(analysis_key.clone()),
        conflict_policy: ConflictPolicy::default(),
        props,
    });

    // (:Analysis)-[:ANALYZES]->(:Repo) — anchors the analysis to the
    // repo it surveys. The Repo node is upserted by `emit_artifact`
    // for every `artifact.*` event in the same bootstrap run, so
    // pointing at it by natural key is safe; the writer's
    // `assert_write_landed` path catches a regression if the Repo
    // node ever fails to land.
    if let Some(repo) = event.context_repo.as_deref().filter(|s| !s.is_empty()) {
        let mut repo_props = BTreeMap::new();
        stamp_display_label(&mut repo_props, repo);
        repo_props.insert("repo".to_string(), Value::String(repo.to_string()));
        patch.nodes.push(NodeOp {
            label: "Repo".to_string(),
            natural_key: repo.to_string(),
            external_id: Some(repo.to_string()),
            conflict_policy: ConflictPolicy::default(),
            props: repo_props,
        });
        patch.edges.push(EdgeOp {
            edge_type: "ANALYZES".to_string(),
            from_label: "Analysis".to_string(),
            from_key: analysis_key,
            to_label: "Repo".to_string(),
            to_key: repo.to_string(),
            props: BTreeMap::new(),
        });
    }
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
    let display = payload
        .as_ref()
        .map(|p| {
            let head = if p.message.is_empty() {
                p.law_id.clone()
            } else {
                format!("{} · {}", p.law_id, p.message)
            };
            format!("[{}] {head}", p.severity)
        })
        .unwrap_or_else(|| format!("Violation {violation_key}"));
    stamp_display_label(&mut props, &clip_display(&display, 96));
    if let Some(p) = payload.as_ref() {
        props.insert("law_id".to_string(), Value::String(p.law_id.clone()));
        props.insert("severity".to_string(), Value::String(p.severity.clone()));
        props.insert("message".to_string(), Value::String(p.message.clone()));
        if let Some(tier) = p.tier {
            props.insert("tier".to_string(), Value::from(tier));
        }
    }
    patch.nodes.push(NodeOp {
        label: "LawViolation".to_string(),
        natural_key: violation_key.clone(),
        external_id: Some(violation_key.clone()),
        conflict_policy: ConflictPolicy::default(),
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
        // Display label = the id itself until spec-13 ships full
        // Law metadata (title / severity / scope). The id alone is
        // already enough to identify the rule in the Nexus browser.
        stamp_display_label(&mut law_props, &law_id.clone());
        patch.nodes.push(NodeOp {
            label: "Law".to_string(),
            natural_key: law_id.clone(),
            external_id: Some(law_id.clone()),
            conflict_policy: ConflictPolicy::default(),
            props: law_props,
        });
        patch.edges.push(EdgeOp {
            edge_type: "OF".to_string(),
            from_label: "LawViolation".to_string(),
            from_key: violation_key.clone(),
            to_label: "Law".to_string(),
            to_key: law_id,
            props: BTreeMap::new(),
        });
    }

    // OBSERVED_IN(LawViolation → Turn|ToolCall) — picks the right
    // MERGE label from the payload's `observed_event_kind`
    // discriminator (added by phase2_graph-observed-in-edge). The
    // schema's allOf/if-then guarantees the discriminator is set
    // whenever `observed_event_id` is set, so we never have to
    // guess — no phantom-node risk via MERGE.
    if let Some(p) = payload.as_ref() {
        if let (Some(observed_id), Some(observed_kind)) = (
            p.observed_event_id.as_deref(),
            p.observed_event_kind.as_deref(),
        ) {
            let to_label = match observed_kind {
                "turn" => Some("Turn"),
                "tool_call" => Some("ToolCall"),
                _ => None,
            };
            if let Some(label) = to_label {
                patch.edges.push(EdgeOp {
                    edge_type: "OBSERVED_IN".to_string(),
                    from_label: "LawViolation".to_string(),
                    from_key: violation_key,
                    to_label: label.to_string(),
                    to_key: observed_id.to_string(),
                    props: BTreeMap::new(),
                });
            }
        }
    }
}

/// Pick the most useful target string for a [`ToolCallPayload`] —
/// the first touched path, then `path` / `file_path` / `pattern` /
/// `command` from the input bag. Returns an empty string when the
/// payload has nothing display-worthy.
fn tool_call_target(p: &ToolCallPayload) -> String {
    if let Some(touched) = p.touched.first() {
        if !touched.path.is_empty() {
            return touched.path.clone();
        }
    }
    tool_call_target_field(p).unwrap_or_default()
}

/// Pull a single labelable string from `input.{path,file_path,pattern,command}`.
fn tool_call_target_field(p: &ToolCallPayload) -> Option<String> {
    let candidates = ["path", "file_path", "filename", "pattern", "command", "url"];
    let obj = p.input.as_object()?;
    for k in candidates {
        if let Some(v) = obj.get(k).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Build the display label for an `agent_call` event. We route those
/// through `emit_tool_call` for graph identity reasons (no separate
/// AgentCall MERGE template today) but still want the label to read
/// `Task: <agent_type> · <description>` so the Nexus browser
/// distinguishes them visually.
fn agent_call_display(event: &EnrichedEvent) -> String {
    let payload: Option<AgentCallPayload> =
        serde_json::from_value(event.redacted_payload.clone()).ok();
    match payload {
        Some(p) if !p.description.is_empty() => {
            format!("Task: {} · {}", p.agent_type, p.description)
        }
        Some(p) => format!("Task: {}", p.agent_type),
        None => format!(
            "AgentCall {}",
            event.event_id.chars().take(12).collect::<String>()
        ),
    }
}

/// Resolve the owning session id for an event.
///
/// Lookup order:
/// 1. `event.session_id` — the dedicated field the classifier worker
///    populates from the canonical envelope's top-level `session_id`.
///    This is the only source that is correct by construction.
/// 2. The redacted payload's `session_id` field — historical fallback
///    for older enriched events written before the dedicated field
///    landed.
/// 3. The event id itself — last-ditch fallback so the graph stays
///    connected. Produces a synthetic single-event session and is
///    the surface symptom of an upstream that forgot to stamp
///    `session_id`.
fn session_id_of(event: &EnrichedEvent) -> String {
    if let Some(s) = event.session_id.as_deref() {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    if let Some(Value::String(s)) = event.redacted_payload.get("session_id") {
        if !s.is_empty() {
            return s.clone();
        }
    }
    let _ = json!(null); // anchor serde_json import for future expansion
    event.event_id.clone()
}
