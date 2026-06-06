# 33 — Timeline & Branch API

> **Status:** 🟢 P3 shipped (CLI + HTTP + MCP + ITs) · **Owner:** Core team · **Depends on:** 30, 31, 32
> **Phase:** phase18_tlb-timeline-branching

## Goal

Public surfaces (HTTP, CLI, MCP) allow operators and agents to query the
bitemporal axis: retrieve events along the timeline at a point in valid
time, list and manage branches, and walk entity lineage chains. The API
surfaces turn spec 30 (schema), spec 31 (classifier), and spec 32
(branch semantics) into operational tools.

## Scope

**In:**

- Eight HTTP routes (timeline, branch list/show/create/merge/abandon,
  entity history/supersession).
- Five CLI subcommands (`timeline`, `branch`, `query`, `history`,
  `supersession`).
- Five MCP tools + extended `cortex_query` (30 → 35 tool registry).
- `/v1/query` extension (backward-compatible `as_of` / `branch` /
  `projects` / `include_history` / `include_future` / `include_branches`).
- All reads use Cypher with sanitized literals (Nexus 2.2.0 parameter
  binding workaround). Writes use same filter.
- Error shape: `{ "reason": "<code>", "detail": "<text>" }`.
- `503 nexus_unconfigured` when daemon has no Nexus client.

**Out:**

- Cross-project axis activation (spec 34).
- Pre-thinking bundle additions (spec 35).
- GUI views (phase18 §7).

## ADR cross-reference

| ADR | locks                                                                |
|-----|----------------------------------------------------------------------|
| 018 | UTC RFC-3339 second storage; day-precision input (`YYYY-MM-DD`) expands to `T00:00:00Z`. |
| 019 | Branch id composite `<project>:<branch>`; regex `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$`; `main` reserved. |
| 021 | Merge strategy `accept\|partial\|discard`; keep-and-link via `MERGED_INTO`. |
| 022 | Abandon requires non-empty reason field.                             |

## HTTP surface

### Timeline routes

#### `GET /v1/timeline/{project}`

Query timeline events at a project scope, optionally filtered by branch,
kind, time bounds, and historical snapshot.

**Query parameters:**

- `branch` (optional, default `<project>:main`) — branch id composite.
- `kind` (optional) — one of the 12 `TimelineKind` discriminators.
- `from` (optional, RFC-3339 / `YYYY-MM-DD`) — lower bound on
  `valid_from_unix`.
- `to` (optional, RFC-3339 / `YYYY-MM-DD`) — upper bound on
  `valid_from_unix`.
- `as_of` (optional, RFC-3339 / `YYYY-MM-DD`) — caps `recorded_at_unix`
  (ADR-018).
- `limit` (optional, default 50) — clamped to [1, 200].

**Response shape:**

```json
{
  "project": "<string>",
  "branch": "<project>:<branch>",
  "filters": {
    "as_of_unix": 1234567890 | null,
    "kind": "<string>" | null,
    "from_unix": 1234567890 | null,
    "to_unix": 1234567890 | null,
    "limit": 50
  },
  "events": [
    {
      "id": "<string>",
      "kind": "<string>",
      "project_id": "<string>",
      "branch_id": "<string>",
      "entity_id": "<string>",
      "valid_from_unix": 1234567890,
      "recorded_at_unix": 1234567890,
      "title": "<string>",
      "summary": "<string>"
    }
  ]
}
```

Events ordered by `valid_from_unix DESC`.

**Error codes:**

- `400 bad_input` — malformed date, invalid kind, unparseable limit.
- `503 nexus_unconfigured` — no Nexus client.
- `502 nexus_error` — Cypher execution failed.

### Branch routes

#### `GET /v1/branch/{project}`

List all branches in a project, ordered by creation time.

**Response shape:**

```json
{
  "project": "<string>",
  "branches": [
    {
      "id": "<project>:<branch>",
      "name": "<string>",
      "parent_branch_id": "<project>:parent" | null,
      "status": "active" | "merged" | "abandoned",
      "merge_strategy": "accept" | "partial" | "discard" | null,
      "created_at": "RFC-3339",
      "created_by": "<string>"
    }
  ]
}
```

**Error codes:**

- `503 nexus_unconfigured`
- `502 nexus_error`

#### `POST /v1/branch/{project}`

Create a new branch forked from `from` at an optional valid-time anchor.

**Request body:**

```json
{
  "name": "<string>",
  "from": "<branch-name>",
  "valid_time": "RFC-3339 | YYYY-MM-DD" | null
}
```

**Validation:**

- `name` MUST match ADR-019 regex `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$`.
- `from` MUST be `main` OR match the regex (unless literal `main`).

**Response shape:**

```json
{
  "branch_id": "<project>:<branch>",
  "parent_branch_id": "<project>:parent",
  "status": "active",
  "result": { ... }
}
```

**Semantics:**

Writes MERGE of parent + child + `FORKED_FROM` edge. Stamps
`status='active'`, `created_by='cortex-api'`, optional `fork_valid_time`.

**Error codes:**

- `400 bad_input` — invalid name or from format.
- `503 nexus_unconfigured`
- `502 nexus_error`

#### `GET /v1/branch/{project}/{branch}`

Show a single branch's full payload.

**Validation:**

- `branch` MUST be `main` OR match ADR-019 regex.

**Response shape:**

```json
{
  "branch_id": "<project>:<branch>",
  "branch": { ... }
}
```

**Error codes:**

- `400 bad_input` — invalid branch format.
- `404 branch_not_found` — branch absent.
- `503 nexus_unconfigured`
- `502 nexus_error`

#### `POST /v1/branch/{project}/{branch}/merge`

Fold a branch into its parent using a merge strategy.

**Request body:**

```json
{
  "strategy": "accept" | "partial" | "discard"
}
```

**Semantics:**

Looks up `parent_branch_id`. Sets `status='merged'`, `merge_strategy`,
`merge_point_event_id` (shape `MRG-<digits-of-timestamp>`). Writes/updates
`MERGED_INTO` edge carrying strategy + merge_point_event_id. Per ADR-021
§1.4, the classifier reads this edge at retrieval time to decide fold-in
behavior on parent retrievals.

**Response shape:**

```json
{
  "branch_id": "<project>:<branch>",
  "parent_branch_id": "<project>:parent",
  "strategy": "accept" | "partial" | "discard",
  "merge_point_event_id": "MRG-...",
  "merged_at": "RFC-3339",
  "result": { ... }
}
```

**Error codes:**

- `400 bad_input` — invalid strategy.
- `404 branch_not_found` — branch absent or has no parent.
- `503 nexus_unconfigured`
- `502 nexus_error`

#### `POST /v1/branch/{project}/{branch}/abandon`

Close a branch without merge, auditable via history.

**Request body:**

```json
{
  "reason": "<string>"
}
```

**Validation:**

- `reason` MUST be non-empty (ADR-022).

**Semantics:**

Sets `status='abandoned'`, `abandonment_reason` on the branch node.

**Response shape:**

```json
{
  "branch_id": "<project>:<branch>",
  "status": "abandoned",
  "abandonment_reason": "<string>",
  "result": { ... }
}
```

**Error codes:**

- `400 bad_input` — empty reason.
- `404 branch_not_found` — branch absent.
- `503 nexus_unconfigured`
- `502 nexus_error`

### Entity routes

#### `GET /v1/entity/{id}/history`

Retrieve all timeline events tagged with an entity, optionally capped at a
historical snapshot.

**Query parameters:**

- `as_of` (optional, RFC-3339 / `YYYY-MM-DD`) — caps `recorded_at_unix`.
- `limit` (optional, default 50) — clamped to [1, 200].

**Response shape:**

```json
{
  "entity_id": "<string>",
  "as_of_unix": 1234567890 | null,
  "events": [
    {
      "id": "<string>",
      "kind": "<string>",
      "branch_id": "<string>",
      "valid_from_unix": 1234567890,
      "recorded_at_unix": 1234567890,
      "title": "<string>",
      "summary": "<string>"
    }
  ]
}
```

Events ordered by `valid_from_unix DESC`.

**Error codes:**

- `400 bad_input` — malformed date.
- `503 nexus_unconfigured`
- `502 nexus_error`

#### `GET /v1/entity/{id}/supersession`

Walk the `SUPERSEDES` chain up to ≤10 hops in both directions
(predecessors + successors).

**Response shape:**

```json
{
  "entity_id": "<string>",
  "lineage": {
    "id": "<string>",
    "label": "<string>",
    "predecessors": [
      { "id": "<string>", "kind": "<string>" }
    ],
    "successors": [
      { "id": "<string>", "kind": "<string>" }
    ]
  }
}
```

**Error codes:**

- `404 entity_not_found` — entity absent.
- `503 nexus_unconfigured`
- `502 nexus_error`

### Extended Query Route

#### `POST /v1/query` (extension)

The existing `/v1/query` orchestrator endpoint now accepts six optional
fields (phase18 §3.3 wedge) to scope retrieval by bitemporal + branching
axes. All fields are `Option<T>` with `skip_serializing_if=is_none`, so
existing callers round-trip unchanged.

**New request body fields (all optional):**

- `as_of` (string, RFC-3339) — wall-clock now if absent.
- `branch` (string, `<project>:<branch>`) — defaults to `<project>:main`.
- `projects` (array of strings) — cross-project axis stays off if absent
  (spec 34 activation deferred).
- `include_history` (boolean) — demote (not drop) historical facts.
- `include_future` (boolean) — demote (not drop) future-dated facts.
- `include_branches` (boolean) — demote (not drop) abandoned branch facts.

Backward-compatibility guarantee: missing fields behave identically to
pre-phase18 behavior.

## CLI surface

Five new subcommands in `cortex-ops`:

| Command | Usage | Options |
|---------|-------|---------|
| `timeline` | `cortex-ops timeline <project>` | `[--as-of] [--branch] [--kind] [--from] [--to] [--limit] [--nexus] [--json]` |
| `branch` (list) | `cortex-ops branch list <project>` | `[--nexus] [--json]` |
| `branch` (show) | `cortex-ops branch show <project> <branch>` | `[--nexus] [--json]` |
| `branch` (create) | `cortex-ops branch create <project> --name <name> --from <parent> [--valid-time <date>]` | `[--nexus] [--json]` |
| `branch` (merge) | `cortex-ops branch merge <project> <branch> --strategy accept\|partial\|discard` | `[--nexus] [--json]` |
| `branch` (abandon) | `cortex-ops branch abandon <project> <branch> --reason "..."` | `[--nexus] [--json]` |
| `query` | `cortex-ops query "<text>" [--as-of] [--branch] [--projects p1,p2] [--api-url] [--intent] [--limit] [--json]` | (POSTs `/v1/query` with phase18 fields) |
| `history` | `cortex-ops history <entity-id> [--as-of] [--nexus] [--limit] [--json]` | |
| `supersession` | `cortex-ops supersession <entity-id> [--nexus] [--json]` | |

All date inputs (`--as-of`, `--from`, `--to`, `--valid-time`) accept
RFC-3339 or `YYYY-MM-DD` (day-precision expands to `T00:00:00Z`).

Implementations:

- `crates/cortex-cli/src/bin/cortex-ops/timeline.rs`
- `crates/cortex-cli/src/bin/cortex-ops/branch_cmd.rs`
- `crates/cortex-cli/src/bin/cortex-ops/query_cmd.rs`

## MCP surface

Five new tools + one extended tool (30 → 35 tool registry count):

| Tool | Signature |
|------|-----------|
| `cortex_timeline` | `(project, branch?, kind?, from?, to?, as_of?, limit?)` — reads `/v1/timeline/{project}`. |
| `cortex_branch_list` | `(project)` — reads `/v1/branch/{project}`. |
| `cortex_branch_show` | `(project, branch)` — reads `/v1/branch/{project}/{branch}`. |
| `cortex_history` | `(entity_id, as_of?, limit?)` — reads `/v1/entity/{id}/history`. |
| `cortex_supersession` | `(entity_id)` — reads `/v1/entity/{id}/supersession`. |
| `cortex_query` | (extended) — six new optional fields `as_of`, `branch`, `projects`, `include_history`, `include_future`, `include_branches` appended to the existing request schema. |

Parameter validation:

- `require_slug` enforces ASCII shape on `project` / `entity_id`.
- `require_branch` enforces reserved `main` + ADR-019 regex on branch
  name.

Implementation: `crates/cortex-mcp-server/src/tools.rs`.

## Backward compatibility

The `/v1/query` extension holds tight to backward-compatibility invariants:

1. All six new fields are `Option<T>`.
2. Serialization uses `skip_serializing_if=Option::is_none`.
3. Missing `as_of` → wall-clock now (existing behavior).
4. Missing `branch` → `<project>:main` (existing behavior).
5. Missing `projects` → cross-project axis off (existing behavior).
6. Existing callers serialize unchanged (no new fields = no new keys in
   JSON).

## Sanitization contract

Every literal value (project, branch, entity_id, reason, kind) passes
through `sanitize_literal`, which drops `'` `"` `\` `\n` `\r`. This
protects inlined Cypher against injection.

## Pinned tests

Gates that lock the HTTP / CLI / MCP surfaces:

**Integration tests:**

- `crates/cortex-api/tests/branch_it.rs` (9 tests) — create, list, show,
  merge accept/partial/discard, abandon, error paths.
- `crates/cortex-api/tests/timeline_it.rs` (6 tests) — timeline query,
  bounds, historical snapshot, empty results.
- `crates/cortex-api/tests/entity_history_it.rs` — entity history query,
  as_of filtering, supersession walk.

**Unit tests:**

- `crates/cortex-api/src/search/timeline_routes.rs::tests` — route
  handlers, error encoding, date parsing, limit clamping.

**MCP validation:**

- `crates/cortex-mcp-server/src/tools.rs::tests::every_tool_descriptor_inputschema_is_valid_json_schema`
  (§4.8) — all five new tools + extended cortex_query have valid JSON
  schemas.

**Live-stack gate:**

- `CORTEX_TIMELINE_IT=1` — full HTTP + Nexus integration (timeline read,
  branch create/merge/abandon, entity history).
