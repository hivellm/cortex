# Proposal: phase11o_vectorizer_demotion_api

## Why

Phase 11j §5 (consolidation pruning daemon) needs to demote raw events
between Vectorizer collections per the spec's tier schedule
(`hot → warm → cold`, 0-7d → 7-90d → 90-365d → expire). The
in-repo audit of `vectorizer-sdk = "3.2"` shows the SDK exposes only:

- `insert_texts` (embeds + writes a fresh vector)
- `get_vector(collection, id)`
- `search_vectors(...)`
- `create_collection` / `delete_collection` / `get_collection_info`

There is no `move_to_collection`, no per-vector `delete_vector`, and
no batch transfer surface. Real demotion would require either:

1. Read-then-reinsert + delete (impossible — `delete_vector` does
   not exist; `delete_collection` is too coarse).
2. A new `move_to_collection(src, dst, ids)` SDK method, mirroring
   what spec 11j §5.2 assumes already exists.

Without the SDK extension, the pruner can only no-op on the vector
side, which defeats the cost model: the warm tier saves nothing if
nothing actually moves into it.

## What Changes

In `vectorizer` / `vectorizer-sdk`:

1. Server-side endpoint: `POST /collections/{src}/vectors/{id}/move`
   with body `{ "destination": "<warm/cold collection>" }`.
   Implementation can be a streaming copy + originating delete
   (server side has both halves).
2. SDK (Rust): `pub async fn move_to_collection(&self, src: &str,
   dst: &str, vector_ids: &[String]) -> Result<MoveReport>`.
3. Companion: `pub async fn delete_vectors(&self, collection: &str,
   ids: &[String]) -> Result<DeleteReport>` (also useful for
   phase 11j §5.3's hard-purge path).
4. Bump `vectorizer-sdk` workspace pin to the released version that
   carries them.

In `cortex` (this repo, picks up after the SDK ship):

5. Implement `crates/cortex-claude-archive/src/pruner.rs` against
   the new SDK methods (phase 11j §5.1 + §5.2).
6. Wire the pruner into the cron schedule (phase 11j §5.4) and the
   `/v1/health/coverage` block (phase 11j §5.7).
7. Land the two ITs (`pruner_it.rs`, `pruner_safety_it.rs`) per
   phase 11j §5.5 + §5.6 — both gated on `CORTEX_PRUNER_IT=1` so a
   missing live Vectorizer never blocks the default test run.
8. `/cortex forget <event_id>` MCP tool (phase 11j §5.3) calls
   `delete_vectors` on every collection that may carry the event
   id; cascades to Meili (`update_documents` to drop fields, or
   `delete_documents` for hard purge), Nexus (`DELETE` on the
   matching node), and Parquet archive (rewrites the partition
   without the row).

## Impact

- Affected specs: `docs/specs/06-embedder.md` (collection-tier
  demotion contract), `docs/specs/19-retention.md` (pruner schedule
  + hard-purge path), `docs/specs/20-mcp-tool-surface.md`
  (`/cortex forget`).
- Affected code (cortex-side): `crates/cortex-claude-archive/`,
  `crates/cortex-mcp-server/`, `crates/cortex-api/src/health.rs`.
- Affected code (vectorizer-side, external repo): `vectorizer/`
  server + `vectorizer-sdk/` Rust crate.
- Breaking change: NO. Additive on both sides; the consolidator
  ships without the pruner today and just leaves source events in
  the hot tier (the previous behaviour).
- User benefit: Vectorizer storage cost stops growing
  monotonically — the warm + cold tiers actually carry the rows
  the schedule promises to demote.

## Blocked on

- `vectorizer` releasing a server build that supports the move +
  delete endpoints.
- `vectorizer-sdk` (Rust) bumping to that build's wire version and
  exposing the matching client methods.

Track via the Vectorizer repo's issue tracker; no ETA owned by
this task.
