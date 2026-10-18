# Proposal: phase31_pre-thinking-audit-trail

## Scope and execution prerequisite

Adapted on 2026-10-17 to the approved standalone architecture in
`.rulebook/tasks/phase36_local-first-core/design.md`.
Execute this task only after `phase36_local-first-core` is complete.
Preserve this task ID; its former full-stack implementation plan is superseded.

Historical plan: Git baseline `2ba3d2f502f3f3955166ac272ae5d8b28c2aea05`,
at this task's existing paths. Rationale:
`docs/analysis/local-first-core-2026-10/02-backlog-review.md`.

## Why

A retrieved memory or successful delivery receipt does not establish that an
agent used it. Cortex needs a durable local trail connecting exact delivered
evidence to independently observed actions, with uncertainty visible. This
supports debugging and measurement without recreating the former API daemon,
Docker workers, governance engine or model trust scores.

## What changes

- Extend the standalone engine with immutable canonical audit records and a
  rebuildable SQLite projection under its local store. Commit canonical records
  before acknowledging audit persistence; reuse the engine's locking, atomic
  publication and recovery contracts.
- Record query/delivery identity, exact repository scope, timestamp, bounded
  bundle summary, delivered memory IDs and immutable revisions/content hashes,
  source references, outcome/error and measured latency. Preserve observed
  origin, session and turn attribution; unavailable identifiers remain absent.
  Do not copy credentials or unbounded prompt/tool payloads.
- Keep delivery and use evidence separate. Expose `prepared_delivery`,
  `observed_related_action`, `explicitly_evidenced_use` and `unknown`
  with supporting references and attribution. These are evidence labels, not an
  automatic progression: temporal adjacency supports correlation only.
- Persist follow-up observations append-only. Link only within authorized scope;
  uncertain links carry their method and limitations. Explicit use requires
  inspectable evidence connecting an action or acknowledgment to the exact
  delivered revision; an agent assertion is labeled as such, not independently
  verified compliance.
- Provide bounded CLI/MCP inspection and embedded GUI views over the same local
  engine. Queries work with the HTTP server stopped; GUI uses the optional
  embedded server. Metrics show evidence-class counts, unknowns, errors and
  latency with denominators; they do not claim causal productivity gains.
- Specify bounded record/query sizes and explicit retention controls. No implicit
  deletion of canonical audit, memories or imported sources. Any retention action
  requires an explicit operator command, dry-run impact and references preserved
  or visibly marked unavailable; never cascade into source data.

## Acceptance and exclusions

Audit entries survive restart and complete index reconstruction. Reconstruction
failure retains the working index. Concurrent writers and retried observations
cannot silently lose or duplicate entries. Scope isolation applies to lookup,
relationships, metrics and GUI results. Errors never become successful delivery
or consumption evidence.

No external daemon, OS schedule, provider call, automatic punishment, trust-score
job or legacy full-stack dependency is introduced. Existing historical data
remains untouched. Documentation, meaningful regression tests, required quality
gates and independent review precede completion.
