# Proposal: phase6g_relevance_meili_body_projection

## Why

`MeiliKeywordLane::project` ([crates/cortex-api/src/meili_lane.rs:175-216](../../../crates/cortex-api/src/meili_lane.rs)) projects each Meili doc onto `LaneHit.text` using the precedence `summary > title > body`. For `kind=artifact` docs — the bulk of what `cortex-bootstrap` writes for code repos — `summary` is empty, `title` is the file path, and `body` carries the real content. The projection therefore stops at `title`, so every artifact hit lands in the response with `text = "crates/cortex-api/src/main.rs"` (the path) instead of the file content.

Empirically verified on 2026-04-28 against the live daemon (pid 57792): `free_search "JWT refresh vectorizer auth" / scope=cortex` returns `crates/cortex-api/src/main.rs` and `docker-compose.yml`, but **does not return `crates/cortex-api/src/vectorizer_lane.rs`** — the only file in the repo that contains `LoginCreds` / `refresh_token`. Meili's BM25 ranking is computing scores from path tokens alone; any term that lives only in code (`JWT`, `refresh_token`, `LoginCreds`, `looks_like_auth_failure`) is invisible to keyword search.

This is the single largest contributor to "the bundle feels weak" complaints when looking up implementation details. Pre-thinking bundles for "how does the vector lane refresh JWT after 401" pulled `meili_loader.rs`, `data.js`, `archive_loader.rs`, `config.rs`, `dashboard.rs` — none relevant. With `body` as the projection source, the same query would surface the file that actually defines the refresh path.

This is not F-007 (extras-stamping; tracked by `phase6b`), not F-001 (per-repo coverage; closed by archived `phase4a`), and not F-005 (RRF score blending; tracked by `phase6c`). It is a **lane-projection** gap: the read-side picks the wrong field of an otherwise correctly indexed document.

Source:
- `crates/cortex-api/src/meili_lane.rs:175-216` — `project()` with the precedence chain that loses `body`.
- `crates/cortex-fulltext/src/builders.rs:80,191` — write-side stamps `body` from `BodySource` selection; data is there.
- `crates/cortex-fulltext/src/body.rs` — body-selection rules.
- 2026-04-28 live-daemon probes documented in this session's chat.

## What Changes

### Precedence change in `MeiliKeywordLane::project`
Replace the `summary > title > body` chain with a kind-aware policy:

| `doc.kind` | Projection precedence for `LaneHit.text` |
|------------|------------------------------------------|
| `artifact` (code/docs files) | `body > summary > title` |
| `decision`, `analysis`, `memory` | `summary > title > body` (unchanged — curated docs always have a `summary`) |
| `turn`, `tool_call`, `agent_call` | `summary > body > title` (unchanged behaviour for capture rows) |
| `law_violation` | `body > summary > title` (the violation message lives in `body`) |
| anything else | current default `summary > title > body` |

The contract becomes: *use the field that actually carries the searchable content*; surface `path` (today the de facto `text` for artifacts) only as a last resort. The orchestrator's snippet renderer already reads `path` separately, so demoting it from `text` does not lose information; it stops it from masking the real body.

### Fallback when every field is empty
If all three fields are empty, return an empty string and **drop** the hit at the orchestrator's degenerate-hit filter (already in place from the dedupe fix, `crates/cortex-api/src/orchestrator.rs`). No change required there.

### Body-byte clamp
`body` can be larger than the per-snippet cap. The Meili lane today does no clamping in `project`; the orchestrator's later trim ladder handles per-snippet bytes. Keep the same shape — projection returns the full body and the trim ladder enforces the cap. Telemetry: log `tracing::debug!` when the projected body exceeds `8 * 1024` bytes so operators can flag oversized chunks against `OVERSIZE_BODY_BYTES` (already exported by `cortex_fulltext`).

### Worker-side guard
The fulltext worker should never write a doc whose `summary`, `title`, AND `body` are all empty. Add a `tracing::warn!` in `crates/cortex-fulltext/src/builders.rs` near the body-selection step (`select_body`) when `chosen.body.is_empty() && summary.is_empty() && title.is_empty()` — that's a write-side bug worth surfacing, separate from this read-side fix. No behaviour change; the worker keeps writing the doc.

### No schema change to the Meili index
The `searchableAttributes` already include `body` ([cortex_storage::fulltext::INDEXES](../../../crates/cortex-storage/src/fulltext.rs)), so Meili already ranks against `body` content. The bug was only in how the lane *projects* the matched doc back into a `LaneHit` for fusion + snippet rendering.

## Impact

- **Affected specs**: [`docs/specs/08-fulltext-indexer.md`](../../../docs/specs/08-fulltext-indexer.md) (document the kind-aware projection contract); [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) (snippet `text` field semantics).
- **Affected code**: `crates/cortex-api/src/meili_lane.rs` (the precedence change); `crates/cortex-fulltext/src/builders.rs` (the empty-body warn).
- **Breaking change**: NO. `LaneHit.text` shape is unchanged; only the chosen source field flips for artifact / law_violation kinds. Callers that read `text` get strictly more useful content.
- **Depends on**: nothing.
- **User benefit**: keyword search starts matching code-side terms that don't appear in file paths (`JWT`, `refresh_token`, function names, struct names, error messages). Every "Cortex didn't find the obvious file" complaint that traces back to artifact-kind docs is closed by this change. Recall@10 is expected to climb materially on the phase6e harness; the actual uplift is the harness's job to quantify.

## Source

- `docs/analysis/relevance/01-findings.md` — to be appended with `F-009 — Meili artifact projection prefers path over body`.
- 2026-04-28 live-stack probes (this session): `cortex_query free_search` returning paths-as-text instead of code content for every artifact-kind hit.
