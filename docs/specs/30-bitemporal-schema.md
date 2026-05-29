# 30 — Bitemporal Schema

> **Status:** 🟢 P0+P1 partially shipped · **Owner:** Core team · **Depends on:** 07, 08, 11, 16
> **Phase:** phase18_tlb-timeline-branching

## Goal

Every retrievable Cortex entity gains a temporal + branching dimension
so retrieval can scope by point-in-time and branch (`as_of` /
`branch`), so superseded facts stop polluting top-K, and so abandoned
exploration paths stop being re-tried by agents. The bitemporal
contract is the load-bearing primitive — the temporal classifier
(spec 31), branching surfaces (spec 32), timeline API (spec 33),
cross-project axis (spec 34), and temporal pre-thinking (spec 35) all
build on it.

## Scope

**In:**

- Bitemporal columns on every entity node label + every Meili doc +
  every Vectorizer payload.
- New `Branch` and `TimelineEvent` graph node labels.
- Seven temporal / branching edge kinds.
- Schema bootstrap constraints + indexes for the new axes.
- Backfill rules for existing rows (default lifecycles, default
  branch, derived `valid_from`).
- Versioned reindex aliases for rollback.
- `cortex-ops migrate-bitemporal` CLI for the one-shot backfill +
  alias cut-over.

**Out:**

- Temporal classifier state-machine (spec 31).
- CLI / HTTP / MCP surfaces for `cortex timeline` / `cortex branch`
  (spec 33).
- Cross-project `CROSS_PROJECT_REF` propagation in retrieval
  (spec 34).

## ADR cross-reference

| ADR | locks                                                            |
|-----|------------------------------------------------------------------|
| 018 | UTC RFC3339 second-precision storage; day-precision CLI input.   |
| 019 | `(project_id, branch_name)` unique pair; regex; `main` reserved. |
| 020 | Cross-project retrieval default OFF; opt-in via `--projects`.    |
| 021 | Branch merge = keep-and-link; classifier-driven fold-in.         |
| 022 | Never delete; archive `superseded` / `expired` after 5 years.    |
| 023 | `SUPERSEDES` / `OBSOLETES` / `EVOLVES_FROM` disjoint semantics.  |

## 1. Column set per entity

Every node label that participates in retrieval (`Decision`,
`Memory`, `Analysis`, `LawViolation`, `Knowledge`, `Learning`,
`Consolidation`, `TopicCard`, `Turn`, `ToolCall`, `Artifact`,
`Symbol`, `Repo`, `Session`, `Branch`, `TimelineEvent`) carries:

| column          | type    | null? | rule                                          |
|-----------------|---------|-------|-----------------------------------------------|
| `project_id`    | String  | no    | Lower-cased `context_repo`; `"unknown"` fallback. |
| `branch_id`     | String  | no    | `"main"` until P1 §2.12 ships per-project branches. |
| `valid_from`    | String  | no    | RFC3339 second-precision (ADR-018).           |
| `valid_to`      | String  | YES   | Absent = still valid.                         |
| `recorded_at`   | String  | no    | When Cortex first persisted the fact.         |
| `superseded_at` | String  | YES   | Absent = still believed.                      |
| `lifecycle`     | String  | no    | `proposed | active | superseded | deprecated | abandoned | merged` |

The Meili + Vectorizer mirrors stamp the same axes as
`*_unix` integer columns (epoch seconds per ADR-018) so range probes
stay cheap.

## 2. Branch node

```text
Branch {
    id:                    String           # composite "<project>:<name>"
    project_id:            String
    name:                  String           # regex per ADR-019
    parent_branch_id:      String | NULL    # NULL only for `main`
    fork_point_event_id:   String | NULL
    fork_valid_time:       String | NULL    # RFC3339 second-precision
    status:                Enum             # active | merged | abandoned
    merge_strategy:        Enum | NULL      # accept | partial | discard
    merge_point_event_id:  String | NULL
    abandonment_reason:    String | NULL
    created_at:            String           # RFC3339 second-precision
    created_by:            String           # agent or operator id
}
```

Reserved name `main` per project; auto-created by §2.12 migration.

## 3. TimelineEvent node

A thin projection over heterogeneous facts:

```text
TimelineEvent {
    id:               String
    project_id:       String
    branch_id:        String
    valid_time:       String                # the wrapped fact's valid_from
    recorded_at:      String
    kind:             Enum                  # 12 discriminators
    title:            String                # ≤ 80 chars
    summary:          String                # ≤ 2 KiB markdown
    ref_entity_id:    String                # pointer to the wrapped entity
    ref_entity_kind:  String                # label of the wrapped entity
    tags:             [String]
}
```

The 12 discriminators: `commit`, `adr`, `decision`, `release`,
`incident`, `learning`, `task_start`, `task_archive`, `branch_fork`,
`branch_merge`, `branch_abandon`, `cross_project_link`.

## 4. Edge taxonomy

```text
SUPERSEDES(from, to)        # `from` REPLACES `to` (ADR-023).
OBSOLETES(from, to)         # `from` deprecates `to` without replacing.
CONFLICTS_WITH(from, to)    # two facts that disagree; resolution = separate ADR.
FORKED_FROM(child, parent)  # branch ancestry.
MERGED_INTO(branch, parent,
            strategy,
            merge_point_event_id)
                            # branch fold-back.
EVOLVES_FROM(from, to)      # non-replacing precursor link.
CROSS_PROJECT_REF(from, to,
                  version_constraint,
                  valid_from, valid_to)
                            # cross-project dependency.
```

Cardinality + classifier rules: ADR-023 §1.6.

## 5. Schema bootstrap

The constraints + indexes the writer ensures on startup
(`crates/cortex-workers/src/graph/schema.rs::SCHEMA_STATEMENTS`):

- Identity constraints: `b:Branch.id IS UNIQUE`,
  `t:TimelineEvent.id IS UNIQUE`.
- Bitemporal composite indexes:
  - `(d:Decision).(project_id, valid_to)`
  - `(d:Decision).(project_id, branch_id, valid_from)`
  - `(d:Decision).(superseded_at)`
  - Same triplet on `Memory`, `Analysis`, `LawViolation` per
    lifecycle-tracking label.
  - `(t:TimelineEvent).(project_id, branch_id, valid_time)`
  - `(b:Branch).(project_id, status)`

Every statement carries `IF NOT EXISTS` so the bootstrap stays
idempotent on re-run.

## 6. Backfill rules

Existing rows lacking the bitemporal stamp follow these defaults
when `cortex-ops migrate-bitemporal` runs (P1 §2.10):

```text
valid_from   = recorded_at OR created_at OR 1970-01-01T00:00:00Z
valid_to     = NULL
branch_id    = "main"
recorded_at  = the first time the writer observed the row
lifecycle    = derived from existing status fields:
                 - Decision.status == "superseded" → "superseded"
                 - Decision.status == "deprecated" → "deprecated"
                 - everything else                 → "active"
project_id   = lower-cased context_repo OR "unknown"
```

ADRs with `status = superseded` get a paired stamp:

```text
target.valid_to       = next_adr.valid_from
target.superseded_at  = next_adr.recorded_at
target.lifecycle      = "superseded"
edge SUPERSEDES(next, target) written
```

## 7. Reindex strategy (versioned aliases)

P1 §2.8 introduces versioned Meili + Vectorizer aliases so the
migration is rollback-safe:

```text
Meili:      cortex-meili-bitemporal-v1
Vectorizer: cortex-vector-bitemporal-v1
```

The migration writes into the v1 alias; when verified it cuts over.
A rollback is `alias --revert` — no data is destroyed.

For phase18 P1, the alias convention coexists with the existing
per-repo `cortex-<slug>-<family>` indexes. The reindex job copies
bitemporal-stamped rows into the v1 alias; the read path reads from
the alias when the temporal classifier is enabled
(`TemporalConfig::enabled = true`, default true).

## 8. Migration CLI (`cortex-ops migrate-bitemporal`)

P1 §2.9 ships the one-shot CLI:

```text
cortex-ops migrate-bitemporal
    [--dry-run]
    [--project <slug>]            # default: every project
    [--batch-size N]              # default 1000
    [--reindex-alias v1]          # default v1
    [--json]                      # JSON report instead of plain text
```

Idempotent: re-running on already-migrated rows is a no-op via
`MERGE` semantics on the bitemporal columns.

## 9. Migration report

Per §2.13 the CLI emits one row per project:

```text
project: cortex
  nodes_updated:           42_117
  edges_created:           1_293       # SUPERSEDES + EVOLVES_FROM backfill
  branches_created:        1           # main only
  anomalies_flagged:       3
  anomalies:
    - decision DEC-091: status="superseded" but no SUPERSEDES target
    - decision DEC-104: created_at < recorded_at; clamped to recorded_at
    - memory  mem-...:  context_repo absent; project_id stamped "unknown"
```

## 10. Out of scope (spec links)

- Temporal classifier state machine — spec 31.
- Branch surfaces (CLI / HTTP / MCP) — spec 33.
- Cross-project axis activation — spec 34.
- Pre-thinking bundle additions — spec 35.

## 11. Test gates

Pinned tests guarantee the contract:

- `graph::schema::tests::phase18_branch_and_timeline_event_constraints_landed`
- `graph::schema::tests::phase18_bitemporal_indexes_landed`
- `fulltext::settings::tests::bitemporal_axis_fields_are_filterable_and_sortable`
- `graph::bitemporal::tests::*` (5 tests).
- `graph::branch::tests::*` (6 tests).
- `graph::temporal_edges::tests::*` (6 tests).
