# Proposal: phase14f_pre-thinking-feedback-and-budget

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-001 + F-005 (both HIGH).

## Why

Two pre-thinking gaps:

1. The pipeline never knows whether its bundle was useful. There is no `POST /api/feedback`, no per-intent quality dashboard. A bundle that is systematically misleading (wrong snippets, stale decisions) cannot self-correct.
2. The 32 KB bundle cap is fixed across all 6 intents. Spec 12 OQ 2 explicitly notes: "A 32-KB cap is a hunch. Once we have eval data, tune per intent." `explain` needs more snippets / fewer decisions; `law_check` needs only violations.

## What Changes

- New endpoint `POST /v1/pre-thinking/feedback` accepting `{ query_id, helpful: bool, files_cited: string[], rating?: 1..5, free_text?: string }`. Persists to a new `pre_thinking_feedback` SQLite table.
- New per-intent budget config in `cortex_config::PreThinkingConfig`. Defaults preserve the current 32 KB; intent-specific overrides documented per intent.
- Metrics histogram `cortex_pre_thinking_bundle_bytes` segmented by intent.
- Implicit feedback signal: track whether the model cited any file from the bundle in its first-100-token output (Jaccard overlap > 0).
- Dashboard `Pre-Thinking Quality` view: per-intent bundle-size distribution + helpful_rate + files_cited_rate.

## Impact

- Affected specs: `docs/specs/12-pre-thinking-injection.md` § Feedback + § Per-intent budget.
- Affected code: `crates/cortex-pre-thinking/src/{budget.rs,metrics.rs}`, `crates/cortex-api/src/{http.rs,feedback.rs}` (new), `crates/cortex-storage/src/metadata.rs` (new table), `gui/src/views/PreThinkingQuality.tsx` (new).
- Breaking change: NO.
- User benefit: bundle quality becomes measurable + tunable; per-intent budgets stop wasting context on irrelevant sections.
