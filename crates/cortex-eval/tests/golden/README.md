# Golden fixture sets — cortex-eval

This directory is the **single canonical fixture tree** for the eval
harness (phase28_retrieval-eval-gate-live §1.1 — the stale duplicate at
the repo root `tests/golden/` was retired on 2026-07-14; the CI
workflow, the binary's `--golden` default, and every doc reference now
point here).

## Files

| File | Suite | Shape |
|------|-------|-------|
| `retrieval.csv` | retrieval | `id, query, repo, expected_paths` |
| `consolidation.csv` | consolidation | `id, consolidation_id, expected_entities, expected_facts` |
| `classification.csv` | classification | `id, envelope_json, expected_kind` |
| `mcp_search.csv` | mcp_search | `id, tool, query, repo, expected_ids` (6 real rows for the verified-working wired tools — m-003 uses the deliberately low-frequency `WebSearch` tool so the ts-sorted top-5 stays stable; see phase28_retrieval-eval-gate-live §1.3 findings for the tools kept dropped: `files_touched` full-archive-scan latency ~30s + repo-filter mismatch, `topic_search` + `law_violations` corpora empty) |
| `access_control.csv` | access_control | `id, clearance_level, compartment_grants, fact_level, fact_compartments, expected, is_acl_admin` — **intentionally synthetic** Bell-LaPadula truth-table (a predicate matrix has no live counterpart to harvest; phase28 §1.4 confirmed) |

> phase0 2026-06-22 — `retrieval.csv` was re-keyed `expected_event_ids` →
> `expected_paths`: the live `/v1/query` returns `results.snippets[]`
> identified by repo-relative `path` (+ `content_hash`), not `event_id`.
> The driver now sends `{query, intent, scope:{repo}, limit}` and reads
> `results.snippets[].path`. retrieval.csv carries real, harvested paths
> (recall@5=1.0, mrr@10=0.47 baseline). consolidation/mcp_search still
> carry `PLACEHOLDER_*` (corpus + per-lane harness re-key pending).

## How to edit

- **Never change row `id` values** — they are stable keys used by CI diffs.
- Add new rows at the bottom of each file.
- `retrieval.expected_paths` uses `;`-delimited repo-relative snippet paths
  harvested from a live `/v1/query`. `mcp_search.expected_ids` uses
  `;`-delimited stable identifiers from the live event store.
- Rows marked `PLACEHOLDER_*` in the expected columns need to be replaced with real values from a live Cortex run before the gate is meaningful.
- After populating real IDs, regenerate the baseline: `cargo run -p cortex-eval -- --suite retrieval --baseline-out baselines/cdc-baseline-v1.json`

## Curation policy

- All rows must represent real user-reported query patterns or known failure modes.
- Minimum 10 rows per suite; long-term curation targets (carried over from the retired phase14c root README): retrieval **100**, consolidation **50**, classification **200**.
- Refresh cadence: per-incident (any post-mortem that surfaces a retrieval / consolidation / classification regression MUST add a golden row reproducing it — that row stays forever) and quarterly (+10–20 rows per quarter for new feature areas).
- Classification rows must cover all Kind variants; aim for ≥2 examples per kind.
- CSV mechanics: RFC 4180 (`""` escapes inner quotes; the harness parses with the `csv` crate, strict mode + `trim=All`); keep `expected_*` lists ≤ 5 entries per row so a single regression doesn't tank the suite score.

## CDC-001 starter seed

These 10 rows per suite were seeded as part of phase17 §1.2 and represent the
CDC-001 gap analysis. Populate `expected_event_ids` from a live Cortex instance
running against the HiveLLM corpus.
