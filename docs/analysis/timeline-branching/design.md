# Design — Timeline & Branching for Cortex

> **Analysis ID:** TLB-001 / Design
> **Date:** 2026-05-24
> **Scope:** Concrete schema, edge taxonomy, query semantics, and integration with the existing Cortex stack (Nexus graph, Meili index, Vectorizer embeddings, pre-thinking pipeline, MCP server).

---

## 1. Core data model

### 1.1 Bitemporal columns on every entity

Every retrievable Cortex entity (`decision`, `adr`, `learning`, `pattern`, `snippet`, `commit_ref`, `code_ref`, `incident`, `release`, `task`, `turn`) gains four time columns:

```
valid_from:     Timestamp (required)        -- when the fact starts being true in the world
valid_to:       Timestamp | NULL            -- when the fact stops; NULL = still valid
recorded_at:    Timestamp (required)        -- when Cortex first persisted the fact
superseded_at:  Timestamp | NULL            -- when Cortex stopped believing this version; NULL = still believed
```

Plus three categorical columns:

```
project_id:     String                      -- e.g. "cortex", "nexus", "vectorizer"
branch_id:      String  (default "main")    -- branch this fact lives on
lifecycle:      Enum                        -- proposed | active | superseded | deprecated | abandoned | merged
```

Existing rows are backfilled with `valid_from = recorded_at`, `valid_to = NULL`, `branch_id = "main"`, `lifecycle = active` so nothing breaks.

### 1.2 Branch node

A new entity kind `branch` in Nexus:

```
id:                    String
project_id:            String
name:                  String                       -- human label
parent_branch_id:      String | NULL                -- NULL only for "main"
fork_point_event_id:   String                       -- the event the branch forked from
fork_valid_time:       Timestamp                    -- valid time at the fork
status:                Enum (active | merged | abandoned)
merge_strategy:        Enum (accept | discard | partial) | NULL
merge_point_event_id:  String | NULL
abandonment_reason:    String | NULL
created_at:            Timestamp
created_by:            String                       -- agent or user id
```

`branch_id = "main"` always exists per project; the root.

### 1.3 Timeline event

A new entity kind `timeline_event` is a thin wrapper that gives heterogeneous facts a uniform timeline view:

```
id:               String
project_id:       String
branch_id:        String
valid_time:       Timestamp
recorded_at:      Timestamp
kind:             Enum (commit | adr | decision | release | incident | learning | task_start |
                       task_archive | branch_fork | branch_merge | branch_abandon | cross_project_link)
title:            String
summary:          String
ref_entity_id:    String       -- pointer to the underlying entity (the ADR, the commit, etc.)
ref_entity_kind:  String
tags:             [String]
```

The timeline event is the queryable timeline projection; the underlying entity stays where it lives.

### 1.4 Edge taxonomy

New temporal/branch edges in Nexus (added to the existing edge palette):

```
SUPERSEDES(from: entity, to: entity)
    -- "from" replaces "to". Used for ADR chains, learning revisions, decision updates.

OBSOLETES(from: entity, to: entity)
    -- "from" makes "to" no longer applicable, without necessarily replacing it 1:1.

CONFLICTS_WITH(from: entity, to: entity)
    -- two facts that disagree; resolution recorded as a separate ADR / decision.

FORKED_FROM(from: branch, to: branch)
    -- branch ancestry.

MERGED_INTO(from: branch, to: branch, strategy)
    -- branch fold-back.

EVOLVES_FROM(from: entity, to: entity)
    -- non-superseding evolution; "to" is a precursor, "from" is the latest revision.

CROSS_PROJECT_REF(from: entity, to: entity, version_constraint)
    -- "Cortex uses nexus-graph-sdk = 2.1": version_constraint = "2.1".
    -- Carries valid_from / valid_to of its own (the constraint may change over time).
```

### 1.5 Worked example

ADR-016 supersedes ADR-014 in project `cortex`. Stored as:

```
adr_014: { project: cortex, branch: main, valid_from: 2025-09-01, valid_to: 2026-03-12,
           recorded_at: 2025-09-02, superseded_at: 2026-03-13,
           lifecycle: superseded }

adr_016: { project: cortex, branch: main, valid_from: 2026-03-12, valid_to: NULL,
           recorded_at: 2026-03-13, superseded_at: NULL,
           lifecycle: active }

edge:    SUPERSEDES(adr_016 -> adr_014)
```

Query "what is Cortex's config policy as of 2026-02-01" returns ADR-014 (valid). Same query as of 2026-04-01 returns ADR-016. Same query today returns ADR-016 plus the supersession chain for context.

---

## 2. Retrieval semantics

### 2.1 Default query

`cortex_query("X")` defaults to:

```
as_of            = now
project_id       = derived from cwd + scope (existing scope_derive())
branch_id        = "main"
include_history  = false
```

This preserves current behavior for naive callers but routes through the temporal classifier.

### 2.2 Temporal classifier (runs after fusion, before rank)

For each candidate `c`:

```
if c.superseded_at IS NOT NULL AND c.superseded_at <= as_of:
    state = SUPERSEDED
elif c.valid_to IS NOT NULL AND c.valid_to <= as_of:
    state = EXPIRED
elif c.valid_from > as_of:
    state = NOT_YET_VALID
elif c.lifecycle == abandoned:
    state = ABANDONED
elif c.valid_to IS NOT NULL AND c.valid_to <= as_of + threshold:
    state = TEMPORAL  # active but near expiry
else:
    state = VALID
```

Action by state:

```
VALID:       pass through, score = base
TEMPORAL:    pass through, score = base * 1.10 (recency boost)
SUPERSEDED:  drop unless include_history = true (then heavy demote)
EXPIRED:     drop unless include_history = true (then heavy demote)
NOT_YET_VALID: drop unless include_future = true (rare, planning queries)
ABANDONED:   drop unless include_branches = true and branch matches
```

### 2.3 Branch resolution

`branch_id = "main"` → include only facts whose `branch_id = "main"` AT the requested `as_of`. If a branch was merged back, its facts are eligible after the merge point.

`branch_id = "feat/spec-11-v2"` → include `main` facts up to `fork_point` AND `feat/spec-11-v2` facts after.

Walking the branch graph: `descendant_branch_walk(target_branch)` returns the chain to root; retrieval unions facts along the chain with the right temporal filters.

### 2.4 Cross-project propagation

Query `cortex_query("X", projects=[cortex, nexus])`:

1. Run retrieval per project.
2. Walk `CROSS_PROJECT_REF` edges from project A's top-K into project B.
3. Re-apply temporal classifier to cross-project hits using the constraint's `valid_from / valid_to`.
4. Fuse and rerank with provenance preserved (`source_project` per candidate).

Default behavior: include the current project only. Cross-project must be opt-in to keep noise low.

---

## 3. Public surfaces

### 3.1 CLI

```
cortex timeline <project>
    [--as-of <date>] [--branch <name>] [--kind commit,adr,...]
    [--from <date>] [--to <date>] [--limit N]
        -> chronological event list, JSON or pretty-print.

cortex branch list <project>
cortex branch show <project> <branch>
cortex branch create <project> --from <branch>@<date> --name <name>
cortex branch merge <project> <branch> --strategy accept|partial|discard
cortex branch abandon <project> <branch> --reason "..."

cortex query "X" [--as-of <date>] [--branch <name>] [--projects p1,p2,...]
cortex history <entity-id>          -- full bitemporal history of an entity
cortex supersession <entity-id>     -- walk SUPERSEDES chain in both directions
```

### 3.2 HTTP API additions

```
GET  /v1/timeline/{project}?as_of=&branch=&kind=&from=&to=&limit=
GET  /v1/branch/{project}                              -- list branches
GET  /v1/branch/{project}/{branch}                     -- branch metadata
POST /v1/branch/{project}                              -- create branch
POST /v1/branch/{project}/{branch}/merge               -- merge
POST /v1/branch/{project}/{branch}/abandon             -- abandon
GET  /v1/entity/{id}/history                           -- bitemporal trail
GET  /v1/entity/{id}/supersession                      -- chain walk
POST /v1/query  body: { q, as_of?, branch?, projects?, include_history?, include_branches? }
```

All existing endpoints stay backward compatible: missing `as_of` = `now`, missing `branch` = `main`, missing `projects` = derived scope.

### 3.3 MCP tools

```
cortex_timeline(project, as_of?, branch?, kind?, limit?)
cortex_branch_list(project)
cortex_branch_show(project, branch)
cortex_history(entity_id)
cortex_supersession(entity_id)
cortex_query  -- extended with as_of, branch, projects optional args
```

### 3.4 Pre-thinking bundle additions

The pre-thinking bundle schema gains:

```
timeline_window: {
  project: String,
  as_of: Timestamp,
  branch: String,
  recent_events: [TimelineEvent]    -- last N events on the active branch+project
}
supersession_overlay: {
  active_decisions: [AdrRef]        -- decisions in lifecycle=active for this scope
  recently_superseded: [(AdrRef, AdrRef)]  -- pairs (new, old) within last K days
}
branch_context: {
  current_branch: String,
  active_sibling_branches: [BranchRef]
  recently_merged: [BranchRef]
}
```

These give the LLM explicit temporal anchors before it sees the query.

---

## 4. Storage and indexing

### 4.1 Nexus (graph)

Schema migration:
- Add bitemporal columns + `project_id` + `branch_id` + `lifecycle` to every entity node label.
- Add the new edge kinds.
- Create indexes on `(project_id, valid_to)`, `(project_id, branch_id, valid_from)`, `(superseded_at)` for fast point-in-time filtering.
- Add a `branch` node label and seed `main` per project.

### 4.2 Meili (lexical)

- Document body unchanged.
- Add filterable attributes: `project_id`, `branch_id`, `valid_from_unix`, `valid_to_unix`, `superseded_at_unix`, `lifecycle`.
- Use Meili filters at query time: `lifecycle = active AND (valid_to_unix IS NULL OR valid_to_unix > <as_of>) AND project_id = <p>`.

### 4.3 Vectorizer (dense)

- Vector payloads gain the same temporal + project + branch fields.
- Pre-filter before ANN search by `project_id` + `branch_id` to keep candidate sets small.
- Temporal classifier runs **after** ANN but **before** rerank.

### 4.4 Bitemporal migration

One-off `cortex-ops backfill-bitemporal` job:

1. For each existing entity: set `valid_from = recorded_at OR created_at OR 1970-01-01`, `valid_to = NULL`.
2. For each ADR with `status = superseded`: set `valid_to = next_adr.valid_from`, `superseded_at = now`, write `SUPERSEDES` edge.
3. For each project root: create `branch:main`.
4. Mark every existing entity `branch_id = "main"`, `lifecycle` derived from existing status fields.

Migration is idempotent and re-runnable.

---

## 5. Audit and observability

- Every temporal classifier decision emits an audit event:
  `temporal_classification(entity_id, state, action, reason, as_of, branch)`.
- `cortex-audit` query lane gains a temporal dimension so debug queries like "why was ADR-014 not in top-5?" return "SUPERSEDED at 2026-03-13, demoted".
- Grafana / observability dashboard adds:
  - % of queries with non-`now` `as_of` (adoption signal).
  - % of candidates filtered by temporal classifier.
  - Branch usage distribution.

---

## 6. Open design questions to confirm before build

1. **Granularity of `valid_time`** — second-precision or day-precision? Most ADRs / decisions only need day; commits need second. Recommend storing `Timestamp` (UTC seconds) but documenting that ADR/decision tooling defaults to day-precision.
2. **Branch identifier scheme** — free-form `(project, name)` strings or hashed IDs? Recommend `(project_id, branch_name)` for human readability + uniqueness constraint.
3. **Cross-project default scope** — opt-in (safer, less noise) vs. opt-out (more recall). Recommend opt-in until eval harness shows benefit.
4. **Branch merge semantics in retrieval** — after merge, do branch facts get rewritten with `branch_id = main`, or kept on branch with merge edges? Recommend keep-and-link (preserves history; classifier handles fold-in).
5. **Retention** — do `superseded` and `expired` facts ever get purged? Recommend never delete; archive to cold storage after configurable age (default 5 years).
6. **Interaction with consolidation** — when a topic card is rewritten, is the old version a `SUPERSEDES` or `EVOLVES_FROM`? Recommend `EVOLVES_FROM` for organic rewrites, `SUPERSEDES` for explicit contradiction.

These are decisions worth recording as ADRs (likely 2–4 ADRs) once the build starts.

---

## 7. Failure modes the design guards against

| Failure | Guard |
|---|---|
| Old ADR ranks above current one | Temporal classifier marks SUPERSEDED, drops or demotes. |
| Fix in Nexus 2.1 silently invalidates Cortex doc | CROSS_PROJECT_REF with version constraint; classifier checks the constraint. |
| Branch experiment leaks into main query results | `branch_id` filter applied at every retrieval lane. |
| Abandoned approach gets retried by an agent | Branch marked `abandoned` with reason; agent sees the reason in timeline lookup. |
| Audit asks "what did we know on date X?" | Bitemporal `recorded_at` filter answers exactly that. |
| Schema change loses historical data | All migrations are additive; backfill is idempotent; no destructive edits. |
