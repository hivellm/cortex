# 12. ADR-011 — Typed LaneHit overlay replaces extras: Props HashMap

**Status**: proposed
**Date**: 2026-05-20
**Related Tasks**: phase13c_lane-trait-typed-projected-hit-adr-011, phase11v_mcp-fine-grained-backend-search

## Context

Every lane (vector / keyword / graph) returns `LaneHit { … extras: Props }` where `Props = BTreeMap<String, serde_json::Value>` (crates/cortex-api/src/lanes.rs:119-147). Overlay correctness — decision_id, supersession status, law_id, turn_id, edge endpoints, contradiction flags — is enforced by stringly-typed lookups scattered across `orchestrator.rs::derive_*` (≥13 distinct `extras.get("…").and_then(|v| v.as_str())` call sites at lines 257, 571, 624, 631, 662-669, 685, 827-843). The current contract lives as a const allow-list (`LANE_EXTRAS_KEYS`, 8 entries) plus an out-of-band convention that lanes "MUST stamp" the right keys when present. A live lane that fails to stamp `decision_id` produces a `LaneHit` that compiles fine, ships, and silently breaks the decisions overlay — exactly the regression observed pre-phase6b. Every new lane (phase11v fine-grained MCP search tools, phase11k governance dual-write) recreates this same shape. The blocked task `phase11v_mcp-fine-grained-backend-search` is the first consumer that cannot ship without the trait-level fix.

## Decision

Adopt a typed Overlay struct as the single load-bearing field on the lane projection. `LaneHit.extras: Props` becomes `LaneHit.overlay: Overlay` where Overlay is a strict struct with optional fields: decision_id, decision_status, superseded_by, turn_id, model, summary, law_id, violation_id, severity, edge_from, edge_to, hops, body_truncated, contradiction_flag, consolidation_grain, topic_id, source (LaneSource enum). Promote `pub trait Lane: Send + Sync` to the canonical lane contract (existing VectorLane, KeywordLane, GraphLane traits become `impl Lane for …`). Every `extras.get("…").and_then(|v| v.as_str())` call site in `orchestrator.rs::derive_*` rewrites to typed `overlay.field` access. Empty-overlay default lets lane impls populate only the fields the upstream document carries; missing fields stay `None` instead of "absent map key". `source: LaneSource` enum replaces the stringly-typed `extras["source"] = "keyword"` convention so the orchestrator's lane-of-origin tie-break is checked at compile time.

## Alternatives Considered

- A) Keep extras: Props; add a runtime validator that lints required keys. Catches missing keys at boot via a self-test, not at compile time. Rejected: validator runs after the bug has shipped to staging; cannot enforce per-lane invariants (KeywordLane should stamp turn_id but not edge_from) without per-lane allow-lists that drift.
- B) Per-lane typed structs (VectorHit / KeywordHit / GraphHit) that the orchestrator pattern-matches. Avoids the unified Overlay but explodes the orchestrator into per-variant branches and prevents future lanes (analytics, semantic-cache) from reusing the fusion logic. Rejected: phase11v alone adds four new lanes; a closed enum is the wrong shape for the growth direction.
- C) Stay on extras + add doc comments + #[deny(missing_docs)] discipline. Cheapest. Rejected: it is what we have today; the pre-phase6b regression already demonstrated that docs do not catch this class of bug.

## Consequences

Positive: Compiler enforces overlay correctness. A lane impl that forgets to populate `overlay.decision_id` shows up as `None` in the orchestrator's pattern match instead of a silent drop. New lanes inherit the contract by construction — no allow-list to keep in sync. `LANE_EXTRAS_KEYS` const + the prose contract in spec-11 §Lane projection contract collapse into the `Overlay` rustdoc. Removes ~13 `as_str()` / `as_u64()` runtime parse calls. Negative: One-time refactor cost — 3 lane impls + orchestrator's six `derive_*` functions + every test that builds a `LaneHit` literal (~1 sprint per proposal). Adding a new overlay field is a struct edit instead of a string key — strictly an upgrade but means a Cargo recompile for any consumer. Neutral: Wire format at `/v1/query` boundary unchanged — ProjectedHit serialises Overlay as a flat JSON object so external callers see the same shape. Internal-only break. Free-form lane metadata that does not belong in the Overlay moves to a separate `debug: BTreeMap<String, Value>` field if needed.
