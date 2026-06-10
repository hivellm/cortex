# Golden fixture sets — cortex-eval

This directory holds the curated CSV fixtures that drive the eval harness.

## Files

| File | Suite | Shape |
|------|-------|-------|
| `retrieval.csv` | retrieval | `id, query, repo, expected_event_ids` |
| `consolidation.csv` | consolidation | `id, session_id, expected_entities, expected_facts` |
| `classification.csv` | classification | `id, envelope_json, expected_kind` |
| `mcp_search.csv` | mcp_search | `id, tool, query, repo, expected_ids` |

## How to edit

- **Never change row `id` values** — they are stable keys used by CI diffs.
- Add new rows at the bottom of each file.
- `expected_event_ids` / `expected_ids` use `;`-delimited ULIDs from the live Cortex event store.
- Rows marked `PLACEHOLDER_*` in the expected columns need to be replaced with real IDs from a live Cortex run before the gate is meaningful.
- After populating real IDs, regenerate the baseline: `cargo run -p cortex-eval -- --suite retrieval --baseline-out baselines/cdc-baseline-v1.json`

## Curation policy

- All rows must represent real user-reported query patterns or known failure modes.
- Minimum 10 rows per suite; target 50.
- Refresh cadence: per-incident (when a retrieval regression is reported) and quarterly.
- Classification rows must cover all Kind variants; aim for ≥2 examples per kind.

## CDC-001 starter seed

These 10 rows per suite were seeded as part of phase17 §1.2 and represent the
CDC-001 gap analysis. Populate `expected_event_ids` from a live Cortex instance
running against the HiveLLM corpus.
