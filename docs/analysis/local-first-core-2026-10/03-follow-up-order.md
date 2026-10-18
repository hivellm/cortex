# Follow-up dependencies and acceptance

This review changes backlog relevance, not the order inside any task checklist.
Old phase numbers are retained for continuity; prerequisites below determine
eligibility for the rewritten follow-ups. Their new checklists start unchecked.

| Work | Execution prerequisite | Required outcome |
| --- | --- | --- |
| `phase36_local-first-core` | Completed and archived on 2026-10-18 | Accepted in `49d5f63`: durable file core, historical import, advisory Jev, CLI/MCP/hooks/embedded GUI, installed acceptance and full quality gates. |
| `phase36_codex-adapter` | Its own concurrent checklist | Correct observed Codex payload/client contracts. Coordinate these contracts with standalone wiring without reintroducing a mandatory daemon. This cleanup does not reschedule that session. |
| `phase28_retrieval-eval-gate-live` | Completed local core | Reproducible local corpus and quality/latency/provenance gates; installed read-only smoke. |
| `phase31_pre-thinking-audit-trail` | Completed local core | Canonical evidence distinguishing delivery from observed use, inspectable after restart/reindex. |
| `phase32_feedback-loop-into-ranking` | Completed local core, evaluation and audit tasks | Attributable persisted feedback, optional bounded ranking effects and measured enabled/disabled comparison. |

Evaluation precedes audit in the retained numeric order; feedback waits for both.
No old service-stack task is executable merely because its historical phase
number precedes phase36. The operator's architecture change provides the scope
for these explicit prerequisite changes.

Transferred acceptance requirements remain inside the appropriate phase36
sections: readable resumable historical recall in 1.3, scoped metadata/index/links
in 1.2, honest CLI/MCP/hooks surfaces in 1.5, and functional installed smoke,
dependency review and actual measurements in 1.7. They do not create permission
to execute a later section early.

No result from the retained follow-ups may claim that Cortex improved agent
behavior using counts of captures, generated graph edges or prepared deliveries
alone. Real use must carry evidence; unavailable observations remain unknown.
