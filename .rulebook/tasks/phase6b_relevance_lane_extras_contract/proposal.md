# Proposal: phase6b_relevance_lane_extras_contract

## Why

The orchestrator's overlay derivation (`derive_decisions`, `derive_similar_turns`, law / model / status overlays) reads from `LaneHit.extras` — but the **live lanes** (`MeiliKeywordLane`, `VectorizerLane`) do not stamp the keys those derivations require. Result: in production, `response.results.decisions` and `response.results.similar_turns` are **always empty**, regardless of whether decisions / turns actually matched the query. Pre-thinking bundles never include the "## Recent decisions you should know about" section.

Only the legacy `MemoryKeywordLane` test double stamps `decision_id` / `turn_id` from seed data, which is why every unit test passes while live bundles arrive empty. This is the single highest-leverage *relevance* gap in the analysis (R1 step 2, closes F-007).

Source: `docs/analysis/relevance/01-findings.md` §F-007; `crates/cortex-api/src/orchestrator.rs:341-368` (`derive_decisions` filters by `extras["decision_id"]`); `crates/cortex-api/src/meili_lane.rs:175-216` (live lane stamps only `extras["source"]`); `crates/cortex-api/src/vectorizer_lane.rs` (same gap on the vector side).

## What Changes

### Lane projection contract (`docs/specs/11-query-api.md`)
Document a stable contract that every `KeywordLane` / `VectorLane` impl MUST honour. The relevant overlay-derivation keys are:

| Key | Source field on the upstream document | Consumed by |
|-----|----------------------------------------|-------------|
| `decision_id` | Meili `_meta.decision_id` / Vectorizer payload `decision_id` | `derive_decisions` |
| `decision_status` | upstream `status` for decision rows | decisions overlay (status badge) |
| `supersedes` | upstream `supersedes[]` | decision detail |
| `turn_id` | upstream `turn_id` | `derive_similar_turns` |
| `model` | upstream `model` (for turn rows) | similar_turns overlay |
| `summary` | upstream `summary` (for turn rows) | similar_turns overlay |
| `law_id` | upstream `law_id` | `derive_violations` |
| `severity` | upstream `severity` (for law_violation rows) | violations overlay |

### `MeiliKeywordLane::project`
Currently stamps only `source = "keyword"` and `score`. Extend to copy every key in the contract from the document body (which the meili_loader already preserves under `_meta`). Use `serde_json::Value::get` chains; missing keys round-trip as absent (overlays gracefully skip). Add a `tracing::debug!` when an `kind=decision` doc lands without a `decision_id` — that is a worker-side bug worth surfacing.

### `VectorizerLane::project_search_result`
Vectorizer payloads are flat objects under `metadata`. Same contract; copy from `metadata.decision_id`, etc. Prefer `metadata.*` over `payload.*` when both exist.

### Compatibility shim
Old test fixtures and the `MemoryKeywordLane` already stamp these keys directly into `extras`. No changes to that code path.

### Regression guard
Add a `cortex_api::lane_contract` test module that, given a synthetic upstream document with every contract key populated, asserts each lane projects the keys onto `LaneHit.extras` 1:1. Drives both lanes through their public `project` functions. This catches future regressions where an SDK bump changes the upstream shape.

## Impact

- Affected specs: [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) (lane projection contract).
- Affected code: `crates/cortex-api/src/meili_lane.rs` (extend `project`); `crates/cortex-api/src/vectorizer_lane.rs` (extend `project_search_result`); new test module `crates/cortex-api/src/lane_contract.rs` (or under `tests/`).
- Breaking change: NO — purely additive on `LaneHit.extras`. Consumers that ignore unknown keys round-trip unchanged.
- Depends on: nothing (can land independently of `phase6a`, but R1 ordering ships them together so coverage uplift is observable).
- User benefit: pre-thinking bundles start carrying the "Recent decisions you should know about" + "Similar past turns" + "Law violations" sections in production. Closes F-007 — the single most-cited "Cortex returned nothing useful" complaint root cause.
