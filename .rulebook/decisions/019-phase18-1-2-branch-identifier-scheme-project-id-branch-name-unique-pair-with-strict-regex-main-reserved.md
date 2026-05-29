# 19. phase18 §1.2 — Branch identifier scheme: (project_id, branch_name) unique pair with strict regex; main reserved

**Status**: proposed
**Date**: 2026-05-29
**Related Tasks**: phase18_tlb-timeline-branching

## Context

Phase18 introduces `branch_id` on every entity and a `branch` node label in Nexus. The branch identifier must round-trip through three writers (Meili filterable attr, Vectorizer payload, Nexus node id) and three operator surfaces (CLI `--branch`, HTTP `?branch=`, MCP arg). Two questions: (a) what is the identifier shape, (b) what does the global id key look like when a branch graph spans projects.</context>
<parameter name="decision">Branch identity is the pair `(project_id, branch_name)`. `branch_name` must match `^[a-z0-9][a-z0-9._/-]{0,62}[a-z0-9]$` — lowercase alphanumeric plus `.`/`_`/`/`/`-`, between 2 and 64 chars, no leading/trailing punctuation. The reserved name `main` exists per project and is auto-created by the migration in phase18 P1 §2.12; operators cannot delete or rename it. The Nexus `branch.id` column stores the composite global id `<project_id>:<branch_name>` (e.g. `cortex:feat/spec-11-v2`) so a single graph query can reach every branch without a join. The composite id is opaque to operators — callers use `--project` + `--branch` independently; only the writer composes the dotted form. Branch names are case-sensitive after the lowercase gate (`feat/x` and `feat/X` cannot coexist; the regex blocks `feat/X`). Path-style branch names with `/` are allowed (`feat/spec-11-v2`) and treated as opaque strings — no hierarchical semantics inferred from the slashes.

## Decision

_No decision recorded._

## Alternatives Considered

- ULID-only branch ids (drop name, autogenerate) — rejected because human operators need to type the branch on the CLI; a ULID is unreadable in audit output and cannot be guessed from context
- (project_id, ULID) with separate human-readable label — rejected as duplication; the regex-constrained name is short enough to serve both roles and the supersession audit reads cleaner with one canonical identifier
- Global branch namespace (no per-project scope) — rejected because two unrelated projects can legitimately use the same branch name (`feat/refactor` in cortex and nexus) without ambiguity, and forcing global uniqueness would require coordination across project teams
- Allow uppercase + spaces (free-form branch labels) — rejected because Meili filter expressions, Vectorizer payload filters, and Nexus property indexes all become quote-sensitive; the regex gate keeps every downstream filter quotation-free
- Git-style branch refs (refs/heads/...) — rejected because Cortex branches are higher-level than git branches; a Cortex branch may span multiple commits / repos / decisions, and aligning with git would create false expectations of fast-forward semantics

## Consequences

Wins: every writer (Meili / Vectorizer / Nexus) stores the same identifier; the composite global id `<project>:<branch>` is self-describing in audit logs; the regex closes the quote-escaping rabbit hole at the filter boundary; reserved `main` ensures every project has a canonical retrieval target without a separate config step. Costs: operators who want a branch name with uppercase or special chars must rename; the migration in P1 §2.12 must enumerate every project that has data (otherwise the auto-create misses repos that only have synthetic envelopes). Reassessment trigger: if a future producer needs hierarchical branch semantics (parent / child relationships from the name shape), add an explicit `branch.parent_branch_id` foreign key rather than parsing the path-style name. The validator lives in `cortex-config::validate::branch_name` and is shared between every wire boundary.
