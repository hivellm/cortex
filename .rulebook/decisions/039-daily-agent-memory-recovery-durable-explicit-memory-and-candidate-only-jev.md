# 39. Daily agent memory recovery: durable explicit memory and candidate-only Jev

**Status**: accepted
**Date**: 2026-10-16
**Related Tasks**: phase34_daily-agent-memory-recovery

## Context

Operator authorized one recovery task ahead of old backlog on 2026-10-16. Sol and Astra source review, existing Claude Code session review, and actual Fable, Grok and Kimi calls agree on explicit durable memory, capture/delivery repairs and lower resource use. Preserve Cortex authority and native memories; use selected ai-memory patterns. Jev supplies bounded candidate judgments, never implicit rules or lifecycle transitions. Acceptance requires exact cross-session recall, source attribution, supersession, restart durability and measured budgets.

## Decision

Keep Cortex as the authority and add a compact decision/lesson projection to
its existing SQLite store. Explicit captures commit before optional inference
or indexing. Preserve provenance, repository boundaries, typed decision
lifecycle, stable IDs, idempotency, revisions, and explicit supersession. Deliver
bounded active context with citations and persistent delivery/feedback evidence.

Disable automatic native-agent memory rewriting before adding new capabilities.
Adopt ai-memory's provenance/versioning and bounded-context patterns without
deploying a second memory service. Provide a lightweight local runtime without
mandatory GPU or bulk indexers. Use pinned Jev only for bounded candidate
judgments; it cannot discard explicit writes, widen scope, promote rules, or
change lifecycle. No OS scheduling or automatic native-file repair is authorized.

Activation clarification: capture defaults to local-only. Per-capture
`classify=true` opts an authorized candidate into external Jev advice. Production
does not attach other stored records as neighbors; candidate consent must not
implicitly disclose memories captured locally. This narrows optional enrichment
without changing durable capture or the accepted memory lifecycle.

Execute only phase34_daily-agent-memory-recovery in its exact checklist order.
The operator's priority exception does not revive unrelated historical backlog.
See docs/analysis/daily-agent-memory-2026-10/05-joint-decision.md and
06-execution-plan.md for actual model participation and ownership.

## Consequences

The basic memory loop remains available during classifier and full-stack
outages. SQLite becomes authoritative for daily memory and must pass restart,
scope, correction, retry, and concurrency tests. Optional mirrors need honest
status rather than an implied indexing guarantee. Captured statements remain
evidence with attribution, not authenticated rules.

Acceptance requires exact cross-session recall, no cross-repository distractors,
immediate successor visibility within the former cache window, persistent
feedback, measured 600 ms prompt delivery and a 1,536-byte memory section, plus
lower observed resource use. A model-provider response alone does not establish
installed harness support or prove an agent consumed delivered context.
