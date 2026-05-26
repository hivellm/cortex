# Proposal: phase11l_nexus-external-ids-migration

Source: [`Nexus/.rulebook/tasks/phase9_external-node-ids/`](../../../../../Nexus/.rulebook/tasks/phase9_external-node-ids/)

## Why

Nexus is shipping first-class **external node identity** under `phase9_external-node-ids`. The reserved `_id` property accepts caller-supplied keys (`sha256:hex`, `blake3:hex`, `sha512:hex`, `uuid:…`, `str:…`, `bytes:hex`) and the storage layer treats it as the canonical, unique-by-construction identity slot. Every `CREATE` learns a new `ON CONFLICT MATCH|REPLACE|ERROR` modifier that lets the caller pick conflict semantics declaratively. As of 2026-05-02 the work is mid-flight: §1 (catalog index), §2 (storage paths), §3 (engine + WAL replay) are green and committed; §4 (Cypher executor branches), §5 (REST/RPC/SDK), §6 (docs), §7 (tail) are pending. The Cortex dependency lands once the SDK + REST surface ship in a Nexus 2.x release.

Cortex today emulates external identity by hand:

- Every node label declares a **synthetic** `natural_key` (or `id`) property — `repo|path|content_hash` for `Artifact`, `repo|language|qualified_name` for `Symbol`, etc. — built by `crates/cortex-workers/src/graph/identity.rs` and stamped into `props["natural_key"]`.
- Every Cypher template under `crates/cortex-workers/cypher/` uses `MERGE (n:Label { natural_key: row.key }) SET n += row.props` to dedupe across re-runs.
- Every label gets a manual `CREATE CONSTRAINT … REQUIRE n.natural_key IS UNIQUE` in `crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS`.
- The patch builder under `crates/cortex-workers/src/graph/analyzer/patch_builder.rs` carries a `UNKNOWN_CONTENT_HASH = "*"` sentinel so cross-artifact `:Artifact` upserts produce a valid wire shape when the sibling's hash isn't known yet — a sentinel that **only works** because `natural_key` is a property and `*` doesn't violate uniqueness against other unknowns the way `_id` would.

This emulation costs us:

1. **Performance** — every `MERGE` is a pattern-match scan that pays the property-lookup cost on every batch; `_id` is an O(log n) (or O(1)) catalog seek.
2. **Idempotency drift** — when two patches produce slightly different `props` maps, the `MERGE` matches on `natural_key` but `SET n += props` may overwrite a deliberately-set property silently. `ON CONFLICT MATCH` (no SET) and `ON CONFLICT REPLACE` (full overwrite) are structurally explicit.
3. **Cross-system joins** — external systems (Vectorizer, Lexum, Synap consumers) cannot join Cortex's graph on a stable identifier without round-tripping through the `natural_key` property; Nexus's `_id` is queryable as a first-class index, including from `GET /nodes/by-external-id/{id}`.
4. **Schema noise** — half of `SCHEMA_STATEMENTS` is `natural_key` uniqueness constraints that the external-id index supersedes structurally. Removing them shrinks the bootstrap surface and the test fixtures.
5. **Sentinel ambiguity** — `UNKNOWN_CONTENT_HASH = "*"` is a soft-degrade that depends on `natural_key`-as-property; under `_id` uniqueness, it would collapse every unknown-hash sibling into one node. The migration forces us to fix it properly (deterministic placeholder per `(repo, path)` until the real hash lands).

The migration is **not urgent** — `MERGE` works today — but it is **inevitable**: once Nexus 2.x ships, the SDK surface for `_id` becomes the documented path and the `natural_key` emulation becomes legacy code that has to be maintained alongside the new index. This task plans the structural changes and the reindex, gated on Nexus phase9 §4-§5 shipping.

This phase depends on `phase11k_graph_correlations` (the static analyzer + patch builder must be stable before we mutate their wire shape) and on Nexus phase9 §4-§5 (the Cypher executor branches and SDK surface must be live).

## What Changes

**No new crates.** Every modification lands inside existing workspace members; the surface change is internal to Cortex.

### §1 — Dependency gate

Pin Nexus SDK 2.x minimum (`nexus-graph-sdk = "2.x"`) once phase9 §5 ships. Verify `_id` semantics against the live Nexus instance via a smoke IT before any Cortex code switches over. Document the gate in `.rulebook/PLANS.md`.

### §2 — `NodeOp` surface

`crates/cortex-workers/src/graph/patch.rs::NodeOp` gains:

- `pub external_id: Option<String>` — when `Some`, the writer uses `CREATE … ON CONFLICT MATCH`; when `None`, falls back to the legacy `MERGE … natural_key` path for one transitional release.
- `pub conflict_policy: ConflictPolicy` — `Match` (default) / `Replace` / `Error`.

The patch builder under `crates/cortex-workers/src/graph/analyzer/patch_builder.rs` populates `external_id` from the existing `natural_key` field (no value change — the same string moves slot). The `props["natural_key"]` stamp stays for one release as a soft fallback so live mid-batch readers never see a node without either form of identity.

### §3 — Cypher templates

Every `crates/cortex-workers/cypher/node_*.cypher` template rewrites:

```cypher
-- before
UNWIND $rows AS row
MERGE (n:Label { natural_key: row.key })
SET n += row.props

-- after
UNWIND $rows AS row
CREATE (n:Label { _id: row.key })
ON CONFLICT MATCH
SET n += row.props
```

Edge templates (`edge_*.cypher`) stay unchanged in shape — they `MATCH` endpoints on the endpoint's identity property; they just transparently benefit from the index seek when that property is `_id`.

### §4 — Schema bootstrap

`crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS` drops the seven `natural_key`-based uniqueness constraints (`artifact_natural_key`, `symbol_natural_key`, `external_package_natural_key`, `unresolved_import_natural_key`, `doc_section_natural_key`, plus any phase11k-introduced equivalents). The external-id index enforces uniqueness automatically. ID-property constraints stay (`session_id`, `turn_id`, `tool_call_id`, `decision_id`, `memory_id`, `analysis_id`, `law_id`, `violation_id`) because those identifiers also serve as the `_id` value — drops are belt-and-braces. The `repo_name` constraint stays (the `name` field is a separate semantic check beyond `_id`).

A doctor check (`crates/cortex-cli/src/ops/doctor.rs`) verifies every emitted `NodeOp` carries an `external_id` once the migration is locked.

### §5 — `UNKNOWN_CONTENT_HASH` sentinel rewrite

`crates/cortex-workers/src/graph/analyzer/patch_builder.rs::UNKNOWN_CONTENT_HASH = "*"` is replaced by a deterministic placeholder format: `pending|{repo}|{path}` (no content_hash slot). The §5.3 stale-edge sweeper from phase11k catches the placeholder when the real hash lands and either (a) re-routes the edges via `delete_edges_by_filter` + a fresh `_id`, or (b) relies on `ON CONFLICT REPLACE` to overwrite once the canonical hash flows in.

### §6 — Bootstrap envelope shape

`crates/cortex-cli/src/bootstrap/graph_static.rs` envelope's `payload.metadata.graph_patch.nodes[*].natural_key` slot becomes `nodes[*]._id`. The archive_loader-side parse path tolerates both shapes for one migration window so a half-replayed archive is never blocked. Bumps the embedded `analyzer_version` constant (`phase11k.1` → `phase11l.1`) so the §5.4 coalescer dedupes correctly across the cutover.

### §7 — Reindex

Two-stage reindex:

1. **Drop** — new admin command `cortex-ops graph drop --confirm` calls Nexus's `DELETE` surface to wipe every `:Artifact` / `:Symbol` / `:Decision` / `:Memory` / `:DocSection` / `:Session` / `:Turn` / `:ToolCall` / `:Repo` / `:ExternalPackage` / `:UnresolvedImport` / `:Spec` / `:Analysis` / `:LawViolation` / `:Knowledge` / `:Learning` / `:Consolidation` node currently in the Cortex graph DB. Idempotent; safe to re-run.
2. **Replay** — re-run `cortex-bootstrap --graph-static` against every indexed repo (`cortex`, `vectorizer`, `nexus`, `synap`, `lexum`, `expert`, `rulebook`, `tml`, `transmutation`, `transmutation-lite`, `vectorizer-sync`, `compression-prompt`, `gui`, `dashboard`, `tests`, `tmldocs`, `hivegpu`, `hivehubcloud`). Boot graph-worker; archive_loader replays everything live captured.

Estimated wall-clock: 15-25 min on the full 17-repo workspace. Disk delta: zero (Nexus reuses the same store; only the index contents change).

### §8 — Dashboard surface

`crates/cortex-api/src/dashboard.rs` graph view colour-coding currently reads `props["natural_key"]` for the canonical node label. After migration: read `_id` (projected via `RETURN n._id`). The `display_label` prop stays unchanged so the human-facing surface is invariant.

### §9 — ADR

`rulebook_decision_create` records the supersession: "Cortex graph nodes carry their identity in Nexus's reserved `_id` slot, replacing the synthetic `natural_key` property convention shipped in phase4-phase11k". Names the quantitative reassessment trigger (Nexus query latency p95 on Artifact MERGE vs. CREATE ON CONFLICT MATCH).

## Impact

- **Affected specs**:
  - `docs/specs/07-graph-writer.md` — §Stable identity rewrite + §Schema bootstrapping shrink.
  - `docs/specs/11-query-api.md` — `MATCH (n {_id: …})` is now an index seek; document the projection shape (`RETURN n._id`).
  - `docs/specs/16-dashboard.md` — graph view reads `_id` not `natural_key`.
  - `docs/architecture.md` §6 — the graph correlation layer's identity story moves from "synthetic composite key" to "Nexus external-id".

- **Affected code (no new crates)**:
  - **Modified**: `crates/cortex-workers/src/graph/{patch,schema,coalescer,mapper,identity}.rs` (NodeOp surface + schema rewrite + coalescer key-extraction), `crates/cortex-workers/src/graph/analyzer/patch_builder.rs` (sentinel rewrite + `_id` slot), `crates/cortex-workers/src/graph/nexus_client.rs` (SDK 2.x call shapes), `crates/cortex-workers/cypher/node_*.cypher` (every node template rewritten), `crates/cortex-workers/src/graph/cypher.rs` (template registry stays; bytes change), `crates/cortex-cli/src/bootstrap/graph_static.rs` (envelope shape change), `crates/cortex-cli/src/ops/doctor.rs` (new `_id`-presence check), `crates/cortex-cli/src/ops/graph_drop.rs` (new), `crates/cortex-api/src/dashboard.rs` (read `_id`), `crates/cortex-api/src/archive_loader.rs` (parse both shapes during the migration window).
  - **Removed (after migration window)**: `props["natural_key"]` stamps, `SCHEMA_STATEMENTS` `natural_key` constraint lines.

- **Deps**: bump `nexus-graph-sdk` workspace dep to `2.x` (exact minor pinned once Nexus phase9 §5 ships).

- **Breaking**: NO for downstream consumers (the dashboard renders the same labels; query intents still resolve). YES for any external tool that read `natural_key` directly off the wire — those callers have to migrate to `_id`.

- **Storage delta**: zero for the catalog (Nexus's external-id index uses 16-32 bytes per node, replacing the equivalent `natural_key` property storage). Dashboard query latency improves estimated 5-15% on `Artifact` lookups (index seek vs. property scan).

- **User benefit**:
  - Idempotent re-ingest at storage level — re-running `cortex-bootstrap --graph-static` on an unchanged repo produces zero new nodes (today: produces dedup hits but still pays the MERGE round-trip).
  - Cross-system joins by hash become first-class — `GET /nodes/by-external-id/sha256:abc…` works against Cortex's graph directly.
  - Cleaner schema bootstrap — seven constraint lines disappear, replaced by Nexus's structural guarantee.
  - Fixes the latent `UNKNOWN_CONTENT_HASH = "*"` ambiguity that today would silently collapse unknown-hash siblings if the constraint were ever tightened.

## Source

Nexus task tree: [`Nexus/.rulebook/tasks/phase9_external-node-ids/`](../../../../../Nexus/.rulebook/tasks/phase9_external-node-ids/) — proposal + tasks.md describe the wire shape, conflict policy, and storage layout this task assumes shipping in Nexus 2.x.

Cortex emulation surface this task supersedes: `crates/cortex-workers/src/graph/identity.rs`, `crates/cortex-workers/cypher/node_*.cypher`, `crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS`, `crates/cortex-workers/src/graph/analyzer/patch_builder.rs::UNKNOWN_CONTENT_HASH`.
