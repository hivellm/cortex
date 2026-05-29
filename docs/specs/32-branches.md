# 32 — Branches

> **Status:** 🟡 P2 partially shipped (scaffold + filter) · **Owner:** Core team · **Depends on:** 30, 31
> **Phase:** phase18_tlb-timeline-branching

## Goal

Branches give Cortex a first-class retrieval scope for exploration
paths. A branch retrieval unions facts along the branch's
ancestry chain back to `main`; an abandoned branch's facts stop
surfacing in default retrievals but stay reachable via
`cortex history` for audit.

## Scope

**In:**

- `Branch` node label (spec 30 §2).
- Branch ancestry walker (§3.6 ← shipped).
- Meili / Vectorizer / Nexus branch filter clauses (§3.5 wiring,
  partially shipped — only Meili helper today).
- Merge strategy classifier-time fold-in per ADR-021 §1.4.
- Branch CLI (`cortex-ops branch list|show|create|merge|abandon`,
  spec 33).

**Out:**

- Branch HTTP / MCP surfaces — spec 33.
- Cross-project branch propagation — spec 34.
- Pre-thinking `branch_context` section — spec 35.

## ADR cross-reference

- ADR-019 (branch identity) — `(project, name)` unique pair;
  regex `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$`; `main` reserved.
- ADR-021 (branch merge) — keep-and-link via `MERGED_INTO`; the
  classifier reads the merge edge + strategy to decide fold-in
  per retrieval.

## 1. Branch identity

```text
id = "<project>:<name>"
```

`name` matches `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$` per
`crates/cortex-workers/src/graph/branch.rs::validate_branch_name`.
The reserved name `main` is auto-created per project in
phase18 §2.12.

## 2. Branch retrieval semantics

A retrieval with `branch_id = "<project>:<name>"` unions facts
along the ancestor chain:

```text
chain = branch_ancestry_chain(project, name, parent_map);
// chain = ["<project>:<name>", ..., "<project>:main"]
```

The Meili filter clause:

```text
branch_id IN ["<project>:<name>", ..., "<project>:main"]
```

The Vectorizer pre-filter mirrors the Meili clause via the
payload's `branch_id` field. The Nexus graph walk filters on
the same property.

Default behaviour when the caller does not pass `branch`:
defaults to `<project>:main` (ADR-019 reserved name).

## 3. Merge fold-in

When a branch is merged into a parent (`MERGED_INTO(child, parent, strategy)`),
the classifier (spec 31) walks the edge for any `branch_id =
<parent>` retrieval at `as_of >= merge_point.valid_time`:

| strategy | classifier behaviour                                          |
|----------|---------------------------------------------------------------|
| `accept` | include every branch fact unchanged                           |
| `partial`| include only branch facts whose `merge_kept = true`           |
| `discard`| include no branch facts on `<parent>` retrievals (audit-only) |

The branch facts are NOT rewritten — `recorded_at` stays on the
original branch event per ADR-021 §1.4. Retrievals against
`branch_id = <child>` still surface the fact directly via the
walker.

## 4. Branch CLI (spec 33 §3.1, pending)

```text
cortex branch list   <project>
cortex branch show   <project> <branch>
cortex branch create <project> --from <branch>@<date> --name <name>
cortex branch merge  <project> <branch> --strategy accept|partial|discard
cortex branch abandon <project> <branch> --reason "..."
```

The CLI binary at
`crates/cortex-cli/src/bin/cortex-ops/branch_cmd.rs` is a thin
wrapper over the types in `crates/cortex-workers/src/graph/branch.rs`
+ `crates/cortex-workers/src/graph/temporal_edges.rs`.

## 5. Pinned tests

`crates/cortex-workers/src/graph/branch.rs::tests` (6):

- `main_branch_factory_yields_composite_id`
- `branch_node_op_carries_all_set_fields`
- `branch_node_op_omits_unset_optional_fields`
- `timeline_event_node_op_carries_all_fields`
- `validate_branch_name_accepts_canonical_shapes`
- `validate_branch_name_rejects_off_shape_inputs`

`crates/cortex-workers/src/graph/temporal_edges.rs::tests` (6):

- `all_seven_edges_are_registered`
- `build_obsoletes_carries_no_props`
- `build_merged_into_stamps_strategy_and_merge_point`
- `build_forked_from_uses_branch_label_on_both_ends`
- `build_cross_project_ref_stamps_bitemporal_props`
- `build_cross_project_ref_with_valid_to_stamps_it`

`crates/cortex-workers/src/temporal/branch_filter.rs::tests` (7) —
see spec 31 §7.
