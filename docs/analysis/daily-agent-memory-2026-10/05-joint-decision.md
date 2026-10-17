# Joint recovery decision

Date: 2026-10-16. Decision: recover the daily memory loop in one bounded task.

## Operator authority and scope

The operator explicitly approved: "Sim: faca recuperacao de memoria primeiro".
This is the one-task exception allowed by LAW-CORTEX-001. The subsequent
instruction restricts work to tasks important to the current project objective;
old backlog items are not automatically reinstated.

The objective is observable behavior: an agent in a later session receives the
correct prior decision or verified lesson before repeating a failed approach.

## Actual participation

| Participant | Mechanism | Contribution |
| --- | --- | --- |
| GPT Sol | Codex subagents | Source audit, Jev contract research, testable design |
| GPT Astra | Codex subagent | Pinned ai-memory source review and adoption tradeoffs |
| Existing Claude Code session cortex-73 | Local peer message, acknowledged in 04 | Runtime evidence, ownership, candidate-only Jev recommendation |
| Claude Fable 5.1 | 9router cc/claude-fable-5-1 | Durable SQLite/FTS memory plus delivery repairs |
| Kimi K3 | 9router kimi/kimi-k3 | Local memory projection, scoped retrieval, restart tests |
| Grok 4.6 | 9router xai/grok-4.6 | Candidate-only Jev, pipeline risk and regression tests |

Grok's first broad request timed out; the smaller retry returned a substantive
review. Fable's response reached its output cap; its architecture and acceptance
sections were received. These are design reviews, not runtime verification.
The existing Claude session owns its coordination reply and Vite process, not
the pre-existing tracked changes. The operator restarted Docker, not Claude.

## Accepted architecture

Keep Cortex as the system of record. Add a compact projection for explicit
decisions and lessons to its existing SQLite storage. Reuse typed decision
status/supersession semantics. Store provenance, explicit repository scope,
stable identity, revisions, sources, and delivery evidence. FTS is a bounded
candidate lookup, not proof that advice is correct.

An explicit write must be durable and immediately available to the daily
memory read path without depending on a GPU, embedding worker, or remote
classifier. Optional full-stack indexes may mirror the evidence. Acceptance,
index completion and actual delivery are separate observable states.

Daily context contains a small set of applicable active memories with stable
IDs and sources. Scope and lifecycle checks precede ranking. Corrections use
an explicit supersession operation; cached context includes a store revision
so an identical prompt cannot keep serving a superseded correction.

Existing unknown callers stay honestly unknown. Client-reported attribution
is not authenticated identity; a model cannot acquire authority by claiming
another agent name. Trusted transport metadata wins where available.

Use Jev only for low-volume memory-candidate judgments: durability, kind,
suggested scope and optionally contradiction hints against a bounded candidate
set. Explicit user/agent writes still persist if Jev is unavailable or disagrees.
Jev cannot broaden scope, silently drop an explicit correction, create project
rules, generate summaries, or supersede records. Pin the tested model, redact
egress, validate typed answers, cap requests and persist usage.

Provide a lightweight daily runtime. Full archival/indexing mode remains
optional and must use bounded incremental work rather than repeated complete
decompression. Existing data and native-agent memory files are preserved.

## Important work versus deferred expansion

Required: stop automatic rewriting of native agent memories; durable scoped
memory; capture/read/MCP contracts; correct prompt delivery; candidate-only Jev;
incremental archival mode; lightweight operation; exact behavioral tests and
independent review.

Not selected: a second permanent ai-memory service, all-event 49-question Jev
enrichment, new GraphRAG work, autonomous rule synthesis, lane-weight learning
without signals, speculative CLI adapters, or unrelated old roadmap features.
No destructive corpus replay or deletion is authorized by this decision.

No adapter stub is accepted. A client path is either implemented and tested
or explicitly reported unsupported; model-provider availability does not prove
a particular CLI lifecycle hook exists.

## Acceptance and limits

Session A records an exact correction. A different agent in session B receives
that correction with provenance under the configured time/byte limits; another
repository and distractors remain absent. An explicit successor replaces it
within the cache window and after process restart. Receipt rows show what was
delivered. Provider outage preserves the basic memory loop. Jev has zero calls
on raw tool events. Resource measurements compare actual runtime configurations.

Models review patches and produce test cases during implementation. The
coordinator verifies changes and gates; model consensus does not waive tests.
