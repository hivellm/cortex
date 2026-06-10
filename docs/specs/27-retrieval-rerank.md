# 27 — Retrieval Rerank (Cross-Encoder)

> **Status:** ✅ P2 shipped · **Owner:** Core team · **Depends on:** 11 (fusion lane), cortex-config ADR-016
> **Phase:** phase17_cdc-code-doc-correlation

## Goal

Insert a cross-encoder reranker (BGE-reranker-v2-m3 via HuggingFace Text
Embeddings Inference) into the spec-11 fusion lane so retrieved candidates
are scored by semantic relevance rather than only by RRF rank. The step is
fail-open: on timeout or service error the pre-rerank fusion order is
preserved and an audit event is emitted.

## Scope

**In:**

- `Reranker` trait in `crates/cortex-workers/src/rerank/mod.rs` with
  `score(query, candidates) -> Result<Vec<f32>, RerankerError>`.
- `Candidate` struct: `doc_id: String`, `text: String`.
- `BgeRerankerV2M3` impl in `crates/cortex-workers/src/rerank/bge_v2m3.rs`
  — HTTP POST `{endpoint}/rerank` using the TEI format, configurable
  timeout.
- `RerankerConfig` in `cortex-config/src/sub.rs` + top-level `Config`
  field `reranker: RerankerConfig`.
- Env knobs: `CORTEX_RERANKER_ENABLED`, `CORTEX_RERANKER_ENDPOINT`,
  `CORTEX_RERANKER_TIMEOUT_MS`, `CORTEX_RERANKER_TOP_K`.
- Orchestrator step: after cross-project propagation, before
  anchor-dedupe/truncate. Sends `top_k_input` (default 100) fused
  candidates; overwrites `LaneHit::score` with the returned logit and
  re-sorts descending.
- Fail-open contract: `RerankerError::Timeout` or any HTTP error → keep
  pre-rerank order, emit `tracing::info!(target: "cortex_audit",
  event = "reranker.fallback", ...)`.
- `Orchestrator::with_reranker(impl Reranker, RerankerConfig)` builder so
  the live binary and tests can wire in the concrete impl without
  changing existing `new()` call sites.
- Integration tests: success path, timeout fallback, disabled-flag
  passthrough (`crates/cortex-api/tests/rerank_it.rs`).

**Out:**

- Latency eval gate (§2.7 `cortex-eval --suite retrieval`): requires a
  live Cortex stack + real golden CSV event IDs. Gated on phase17 §1.2
  refresh.
- TEI service deployment / Dockerfile: operator-managed, not shipped in
  this phase.
- Reranker caching / local ONNX inference: future work.

## Config defaults

| Field | Default | Env override |
|-------|---------|-------------|
| `enabled` | `true` | `CORTEX_RERANKER_ENABLED` |
| `endpoint` | `None` (inactive) | `CORTEX_RERANKER_ENDPOINT` |
| `top_k_input` | `100` | `CORTEX_RERANKER_TOP_K` |
| `timeout_ms` | `500` | `CORTEX_RERANKER_TIMEOUT_MS` |

When `endpoint` is `None` the reranker step is skipped even if
`enabled = true`, so operators can ship the feature flag before standing
up the TEI service.

## Pipeline position

```
rrf_fuse(vector + keyword + graph)
  → temporal classifier  (phase18 §3.3)
  → cross-project prop.  (phase18 §5.2)
  → [RERANKER STEP]      ← this spec
  → anchor-dedupe
  → fused.truncate(req.limit)
  → snippet assembly
```

## Fail-open contract

On `RerankerError::Timeout` or `RerankerError::Http`:

1. Pre-rerank fusion order is preserved — no mutation to `fused`.
2. A `tracing::warn!` at level `warn` surfaces the error in operator logs.
3. A structured `tracing::info!(target: "cortex_audit", event =
   "reranker.fallback", reason = %, query_id = %)` is emitted for the
   audit pipeline to capture.

## TEI request format

```
POST {endpoint}/rerank
Content-Type: application/json

{
  "query": "<the rewritten vector query>",
  "texts": ["<candidate 0 text>", "<candidate 1 text>", ...],
  "return_text": false
}
```

Response: `[{"index": N, "score": 0.987}, ...]` — items in descending
score order with the original input index. The impl reconstructs a
score vec aligned to the input slice order.

## Audit event

```
event = "reranker.fallback"
reason = "<error message>"
query_id = "<uuid>"
```

Emitted only on fallback. Normal operation produces no audit event (low
cardinality, no signal needed for the success path).
