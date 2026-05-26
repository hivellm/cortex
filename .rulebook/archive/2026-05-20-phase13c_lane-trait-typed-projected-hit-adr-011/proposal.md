# Proposal: phase13c_lane-trait-typed-projected-hit-adr-011

Source: `docs/analysis/rework/04-architecture.md` §A.3; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase A.3.

## Why

The lane contract is stringly-typed: each lane returns `ProjectedHit { extras: HashMap<String, Value> }`. Overlay correctness (decision_id, supersession, contradiction-flag, etc.) is enforced by lookups like `extras.get("decision_id")` scattered across `orchestrator.rs::derive_*`. Live lanes did not stamp `decision_id` until phase6b; the same shape will recur on every new lane until the contract is typed.

Phase A.3 unblocks the active `phase11v_mcp-fine-grained-backend-search` task, which will land as the first consumer of the typed trait.

## What Changes

- New ADR-011 — "Typed `ProjectedHit` replaces `extras: HashMap<String, Value>` lane contract".
- New trait `cortex_api::lanes::Lane`:
  ```rust
  #[async_trait]
  pub trait Lane: Send + Sync {
      fn name(&self) -> &'static str;
      async fn search(&self, q: &Query, scope: &Scope) -> Result<Vec<ProjectedHit>>;
  }
  ```
- New typed `ProjectedHit { event_id, score, payload, overlay: Overlay }` where `Overlay` is a strict struct with optional fields (`decision_id`, `superseded_by`, `contradiction_flag`, `consolidation_grain`, ...).
- All `extras.get(...)` calls in `orchestrator.rs::derive_*` rewritten to typed access.
- Empty-overlay regression test covers ≥3 lanes.

## Impact

- Affected specs: `docs/specs/11-query-api.md` § Lane contract; ADR-011.
- Affected code: `crates/cortex-api/src/lanes.rs`, `crates/cortex-api/src/types.rs` (ProjectedHit), `crates/cortex-api/src/orchestrator.rs` (overlay derivation), each lane impl (vector / keyword / graph).
- Breaking change: INTERNAL — no wire-format change at the `/v1/query` boundary.
- User benefit: compiler enforces overlay correctness; new lanes (phase11v search tools) ship without overlay drift bugs.
