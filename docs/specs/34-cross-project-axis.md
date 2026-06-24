# 34 — Cross-Project Axis

> **Status:** 🟢 P4 shipped (backfill + propagation + IT-pinned eval gate) · **Owner:** Core team · **Depends on:** 30, 31, 33
> **Phase:** phase18_tlb-timeline-branching

## Goal

Cortex can now resolve cross-project dependencies and surface them into
retrieval results with full temporal validity and provenance tracking.
The backfill layer ingests manifest (Cargo.toml, package.json, gui/package.json)
and ADR (decision/*.md) sources; the propagation layer re-applies the temporal
classifier to constrain visibility by valid_from/valid_to, then fuses
cross-project references with the active project's retrieved facts. This
enables agents to navigate the HiveLLM ecosystem (Cortex → Nexus → Vectorizer → Synap → Lexum → Expert) without explicit disambiguation.

## Scope

**In:**

- `CROSS_PROJECT_REF` edge taxonomy (spec 30 §4): carries version_constraint + bitemporal validity window (valid_from, valid_to; absent → still active).
- Manifest backfill source: Cargo.toml / package.json / gui/package.json. A sibling-dependency registry maps SDK crate / npm package names to HiveLLM project_id (nexus-graph-sdk → nexus; vectorizer-sdk → vectorizer; synap-sdk → synap; @hivellm/* npm variants; lexum, expert).
- ADR backfill source: `.rulebook/decisions/*.md` scanned for project mentions with version regex `(?i)\b<proj>\b[^\n0-9]{0,16}?v?(\d+\.\d+(?:\.\d+)?)`. Decision key derived as `ADR-{:03}` from filename leading digits. MATCH-guarded write (no orphan stubs if Decision absent).
- CLI: `cortex-ops backfill-cross-project [--root .] [--project cortex] [--nexus] [--dry-run] [--json]`. Branch edges MERGE-upserted (idempotent). Deduped output (identical (from, to, version) collapsed to one edge).
- Propagation wedge: `Orchestrator.propagate_cross_project` runs after temporal classifier (spec 31) wedge, before anchor-dedupe. Gated on CrossProjectConfig.enabled AND non-empty request.projects. Walks active project's `<active>:main` CROSS_PROJECT_REF edges, filters to requested siblings, converts valid_from/valid_to to epoch seconds, re-applies temporal classifier with the constraint window (valid_to before as_of → EXPIRED → dropped), stamps source_project provenance, fuses survivors by adjusted score, emits cross_project_propagation audit envelope (active_project / requested / discovered / kept / propagated / dropped).
- Whitelisted graph template `cross_project_ref`: query wraps CROSS_PROJECT_REF traversal with project/version filtering, surfaces version_constraint / valid_from / valid_to in extras, derives source_project from edge_to.
- Configuration: CrossProjectConfig.enabled (default **false** per ADR-020), max_hops (default **1**). Env: CORTEX_CROSS_PROJECT_ENABLED, CORTEX_CROSS_PROJECT_MAX_HOPS.
- Audit envelope: cross_project_propagation carries active_project / requested_projects / discovered_edges / kept_edges / propagated_results / dropped_results.

**Out:**

- Per-entity sibling-corpus fan-out (running keyword/vector lanes against each sibling project's collections and fusing those entity sets) — documented boundary; requires per-project lane orchestration (natural next step).
- Cross-project query corpus (labeled training set for MRR-delta evaluation) — eval gate blocked on operator-owned golden-set (same blocker class as spec 31 §3.8, spec 32 phase14c).
- Default-on flip — ADR-020 keeps opt-in pending eval evidence.

## ADR cross-reference

| ADR | locks                                                            |
|-----|------------------------------------------------------------------|
| 018 | UTC RFC3339 second-precision storage; epoch seconds in transit.  |
| 020 | Cross-project retrieval default OFF; opt-in via `--projects`.    |
| 023 | `CROSS_PROJECT_REF` disjoint from SUPERSEDES / OBSOLETES / EVOLVES_FROM. |

Spec 30 §4 edge taxonomy; spec 31 classifier re-applied during constraint window; spec 33 extended `/v1/query` with `projects` parameter.

## Backfill

### Two source families

#### Manifest dependencies

Cortex scans three manifest files at backfill time:
- `Cargo.toml` (Rust crate version)
- `package.json` (Node main package version)
- `gui/package.json` (Node GUI package version)

A sibling-dependency registry maps:
- Crate names (nexus-graph-sdk, vectorizer-sdk, synap-sdk) → project_id.
- npm package names (@hivellm/nexus, @hivellm/vectorizer, @hivellm/synap, @hivellm/lexum, @hivellm/expert) → project_id.

For each resolved (project, version), writes:
```
MATCH (from:Branch{id: "<this_project>:main"})
MATCH (to:Branch{id: "<dep_project>:main"})
MERGE (from)-[r:CROSS_PROJECT_REF{
  version_constraint: "<version>",
  valid_from: "<now>",
  valid_to: null
}]->(to)
```

**Invariant (SHALL):** The MATCH on `to` MUST NOT fail silently. If the target project's main branch is absent, the edge is written MATCH-guarded so the write is a no-op, never a stub.

#### ADR mention extraction

The backfill scans `.rulebook/decisions/*.md` for project mentions. A linear regex
`(?i)\b<proj>\b[^\n0-9]{0,16}?v?(\d+\.\d+(?:\.\d+)?)` (case-insensitive, non-greedy, up to 16 non-digit chars between project name and version) extracts (project, version) pairs.

Decision key is derived: `ADR-{:03}` from the filename's leading digits. For each resolved (decision_key, project, version), writes:
```
MATCH (from:Decision{id: "<decision_key>"})
MATCH (to:Branch{id: "<dep_project>:main"})
MERGE (from)-[r:CROSS_PROJECT_REF{
  version_constraint: "<version>",
  valid_from: "<recorded_at>",
  valid_to: null
}]->(to)
```

**Invariant (SHALL):** The MATCH on both `from` and `to` MUST be present. If either is absent, the edge write is a no-op (guarded by MATCH), never a stub or error.

### Edge shape

```text
CROSS_PROJECT_REF {
  version_constraint:  String  # parsed from manifest/ADR
  valid_from:          String  # RFC3339 second-precision
  valid_to:            String | NULL  # NULL = still active
}
```

### CLI: `cortex-ops backfill-cross-project`

```
cortex-ops backfill-cross-project
    [--root .]              # codebase root (default current directory)
    [--project cortex]      # project slug (default inferred from context)
    [--nexus]               # Nexus connection string (default from env)
    [--dry-run]             # simulate, print edge count, no writes
    [--json]                # JSON report instead of plain text
```

Output (plain or JSON):
```
manifest_edges_discovered: 3
adr_edges_discovered:      7
deduplicated_total:        10
edges_written:             10 (0 if --dry-run)
```

Sample live dry-run: manifest_edges=3 (nexus 2.1 / synap 0.12 / vectorizer 3.3.0), adr_edges=7, deduped_total=10.

**Invariant (MUST):** The CLI MUST be idempotent. Re-running on already-backfilled rows (MERGE semantics on the edge properties) is a no-op and emits zero changed rows.

**Invariant (MUST):** Deduplication collapses identical (from, to, version_constraint) tuples to a single edge write.

## Propagation

### Whitelisted graph template: `cross_project_ref`

```cypher
MATCH (a:Branch)-[r:CROSS_PROJECT_REF]->(b:Branch)
WHERE a.id CONTAINS $q
RETURN
  a.id,
  b.id,
  type(r),
  1,                          # result weight placeholder
  "cross_project_ref",        # label
  r.version_constraint,       # extras: version
  r.valid_from,               # extras: valid_from
  r.valid_to                  # extras: valid_to
LIMIT 50
```

The template surfaces version_constraint / valid_from / valid_to into `extras` field. The source_project is derived from edge_to: `<project>:main` → `<project>`.

### Propagation wedge

**Trigger:** Orchestrator.propagate_cross_project runs after spec 31 temporal classifier wedge, before anchor-deduplication, when **both** conditions hold:
1. CrossProjectConfig.enabled == true (default false).
2. Request.projects is non-empty (user explicitly requested cross-project scope).

**Algorithm:**

1. Walk active project's `<active_project>:main` outbound CROSS_PROJECT_REF edges.
2. Filter to edges where edge_to's project_id ∈ request.projects.
3. For each kept edge:
   a. Convert valid_from / valid_to to epoch seconds (per ADR-018).
   b. Re-apply temporal classifier (spec 31) with the constraint window: if valid_to is present AND valid_to_unix < request.as_of_unix, mark as EXPIRED and drop.
   c. For survivors: stamp source_project = edge_to's project_id; collect the edge metadata (version_constraint, valid_from, valid_to).
4. Fuse collected cross-project references into the result set by adjusted score (dedupe by destination, keep highest score).
5. Emit cross_project_propagation audit envelope: { active_project, requested_projects, discovered_edges, kept_edges (after temporal filter), propagated_results (final fused count), dropped_results (temporal + dedup casualties) }.

### Gating

The propagation is gated on two conditions:
- **Config gate:** CrossProjectConfig.enabled (environment: CORTEX_CROSS_PROJECT_ENABLED; default **false**).
- **Request gate:** request.projects array is non-empty (omit or pass [] to disable).

Per ADR-020, the feature remains opt-in until eval evidence (§5.4) justifies flipping the default.

## Configuration

### CrossProjectConfig struct

```rust
pub struct CrossProjectConfig {
    pub enabled: bool,        // default: false (ADR-020)
    pub max_hops: u32,        // default: 1 (spec design §2.4)
}
```

### Environment variables

| Variable                              | Type | Default |
|---------------------------------------|------|---------|
| `CORTEX_CROSS_PROJECT_ENABLED`        | bool | false   |
| `CORTEX_CROSS_PROJECT_MAX_HOPS`       | u32  | 1       |

Parse via `cortex_config::CrossProjectConfig::from_env()`.

## Scope boundary

The current implementation surfaces cross-project **references** (Branch→Branch edges with version + temporal provenance) into the active project's retrieval result set. This allows agents to see that project A depends on version 2.1 of project B.

The natural next step — per-entity sibling-corpus fan-out — is **not yet shipped**. This would mean:
1. Running keyword/vector lanes against each sibling project's (Decision, Memory, Analysis, etc.) collections in parallel.
2. Fusing entity results from all projects by fused score.
3. Stamping provenance (source_project) on every fused candidate.

This requires per-project lane orchestration and is deferred to a follow-on task.

**Phase22 §4.4 measurement (2026-06-24):** Cross-project eval rows r-015..r-018 were added
to the labelled golden corpus. These rows target cortex-internal documentation (embedding
provider, graph writes, opt-in policy, propagation filter) and do not require sibling-project
data retrieval. The MRR-delta measurement for cross-project ON vs OFF = 0% — no sibling
projects (Nexus, Vectorizer) are indexed in this Cortex instance, so no CROSS_PROJECT_REF
edge contributes additional results. The functional gate (source_project provenance stamped on
all propagated hits) is SATISFIED by `cross_project_it.rs` 4/4. ADR-020 remains opt-in pending
eval evidence from a corpus with live sibling-project indexing.

## Pinned tests

Gates that lock the backfill + propagation contract:

**Unit tests — backfill logic:**

- `crates/cortex-workers/src/graph/cross_project.rs::tests` (7 tests) — manifest regex parsing, ADR extraction, edge shape, MATCH-guard invariants.
- `cortex-ops backfill_cross_project` unit tests (2 tests) — deduplication, idempotency, CLI argument parsing.

**Unit tests — propagation logic:**

- `crates/cortex-api/src/search/orchestrator.rs::cross_project_propagation_tests` (3 tests) — temporal constraint window application, version_constraint derivation, audit envelope shape.

**Integration tests:**

- `crates/cortex-api/src/search/lanes/nexus_graph_lane.rs::cross_project_ref_template_resolves` — whitelisted template query returns correct edge shape + version metadata.
- `crates/cortex-api/tests/cross_project_it.rs` (4 tests) — end-to-end: backfill manifest edges, propagate via `/v1/query` with `projects` parameter, verify version_constraint + source_project in response, validate temporal filtering (valid_to before as_of drops the edge).

**Eval gate (§5.4, IT-pinned):**

- `crates/cortex-api/tests/cross_project_it.rs` (4/4) — functional provenance gate:
  disabled → no propagation, enabled+in-window → source_project stamped, stale
  valid_to → dropped, unrequested sibling → filtered. Gate PASSED.
- Live MRR-delta gate: 0% on phase22 §4.4 corpus (see note above). Full MRR-delta
  measurement deferred until sibling projects (Nexus, Vectorizer) are indexed locally.
