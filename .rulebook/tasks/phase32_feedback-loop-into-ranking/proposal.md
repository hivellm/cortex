# Proposal: phase32_feedback-loop-into-ranking

## Why
Recorded feedback should support measurable retrieval improvements without
misattributing a negative bundle verdict to every result. The old plan depended
on service lanes and RRF fusion that the approved standalone core replaces.
Its source schema was bundle-level and lacked repository/lane attribution;
absence of a citation is not proof that a record was unhelpful.

## What Changes
Keep this task ID, replace its unstarted implementation scope with local,
provenance-backed feedback and an optional bounded ranking experiment.
Execution requires completed `phase36_local-first-core`,
`phase28_retrieval-eval-gate-live` and `phase31_pre-thinking-audit-trail`.
Do not start this follow-up merely because its phase number is lower.

- Persist feedback as canonical local records, scoped to exact repository,
  query/delivery identity and, when explicitly identified, record ID/revision.
- Validate that the referenced delivery and record belong to the same scope.
  Bundle-level feedback remains bundle-level; unmatched legacy feedback is
  inspectable but ineligible for ranking updates.
- Preserve explicit helpful/unhelpful/unknown judgments and source attribution.
  Heuristic correlations, missing citations and Jev labels cannot become
  explicit feedback or change source authority.
- Add an optional deterministic adjustment with documented sample threshold,
  decay and bounds. Replaying the same feedback must not amplify its weight.
  Keep adjustment disabled by default, explain contributions and allow rollback
  to baseline without deleting feedback or modifying canonical memories.
- Evaluate enabled versus disabled ranking on the locked local corpus before
  recommending enablement. Report improvements and regressions honestly;
  inconclusive or harmful results leave the default disabled.

## Historical Context
The original, unstarted plan remains at commit
`2ba3d2f502f3f3955166ac272ae5d8b28c2aea05` under this same task path.
Review: [backlog disposition](../../../docs/analysis/local-first-core-2026-10/02-backlog-review.md).
No service-lane weighting, trust-score engine or background worker is carried
forward as a requirement.

## Impact
- Affected specs: task-local `retrieval` delta for the standalone engine.
- Affected code: local canonical records/index, retrieval explanation and
  CLI/MCP/embedded GUI feedback surfaces delivered by phase36.
- Compatibility: disabled mode preserves baseline result identity/order/scores.
- User benefit: inspectable evidence for ranking changes without automatic
  promotion, suppression or punishment of memories.
