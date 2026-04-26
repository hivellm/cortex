# 08 — Full-text indexer (Meilisearch client)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 02

## Goal

Index every enriched event's searchable text into **Meilisearch** with typo tolerance, faceted filters, and snippet highlighting — so the query API (spec 11) has a fast keyword lane alongside the vector and graph lanes. This spec is the client: index configuration, document mapping, batched writes, idempotency. Meilisearch operations (install, backup) are out of scope.

## Scope

**In:**
- Worker consuming `cortex.events.enriched` + `cortex.events.embedded`.
- Meilisearch index layout (one index per collection family; mirrors embedder's collection boundaries).
- Document schema: shared core + per-kind extensions.
- Searchable/filterable/sortable attribute configuration.
- Client: HTTP batched upsert, retry, backoff, idempotent upsert.
- Dedup by `document_id` (same content → no-op write).
- Synonyms, stop-words, and ranking-rules configuration (versioned).
- Telemetry.

**Out:**
- Query surface (spec 11 — read path is the orchestrator's job).
- Index analytics / query-log mining — future.
- Meilisearch deploy/ops (docker-compose in spec 03).
- Eventual migration to Lexum — tracked as an open question in architecture §12.

## Inputs / Outputs

### Trait

```rust
#[async_trait]
pub trait FulltextIndexer: Send + Sync {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport>;
}

pub struct IndexReport {
    pub documents_upserted: u32,
    pub documents_deduped: u32,                  // same doc_id + same content_hash already present
    pub by_index: BTreeMap<String, u32>,
    pub latency_ms: u32,
}
```

### Document schema

Shared core (present on every document):

```jsonc
{
  "id": "<doc_id>",                              // see identity section
  "event_id": "01HXYZ...",
  "kind": "tool_call.edit",
  "content_hash": "sha256:...",
  "ts": 1713369600000,                            // ms since epoch (filterable & sortable)
  "repo": "Vectorizer",
  "path": "src/index/hnsw/mod.rs",
  "topics": ["code", "refactor"],
  "severity": "notable",
  "pii_risk": "low",
  "summary": "Refactored HNSW ef_search...",      // classifier summary if present
  "title": "hnsw_ef_search",                      // symbol for code, H1 for docs, first 80 chars otherwise
  "body": "pub fn hnsw_ef_search(k: usize) -> ... ",  // the primary searchable text
  "language": "rust"
}
```

Per-kind extensions live under `ext.<kind>`:

```jsonc
"ext": {
  "tool_call": { "tool_name": "Edit", "status": "ok" },
  "decision": { "status": "accepted", "supersedes": "DEC-0042" },
  "law_violation": { "law_id": "LAW-007", "tier": 3 }
}
```

Missing extensions are absent — no null-padding.

### Indexes (per-kind families, mirrors spec 06)

| Index name              | Holds                        | Typical size (live, post-bootstrap) |
|-------------------------|------------------------------|-------------------------------------|
| `cortex-code`           | `tool_call.*`, `artifact.code` | ~500 MB                            |
| `cortex-docs`           | `artifact.doc`, docs chunks   | ~400 MB                            |
| `cortex-decisions`      | `decision.*`                   | ~20 MB                              |
| `cortex-turns`          | `turn.*`                       | ~800 MB                             |
| `cortex-governance`     | `law`, `law_violation`         | ~10 MB                              |
| `cortex-misc`           | everything else                | varies                              |

Deployment namespace prefix (default `cortex-`) matches spec 06.

### Identity

```
doc_id = event_id                              // live events
doc_id = "bootstrap:" + repo + ":" + path + ":" + content_hash   // bootstrap artifacts
```

- `doc_id` is stable across re-runs.
- Meilisearch upsert is `doc_id`-keyed, so retries and bootstrap replays are no-ops.

## Design

### Index configuration (per index, set at `ensure_index`)

```jsonc
{
  "searchableAttributes": ["title", "body", "summary", "path", "topics", "ext.tool_call.tool_name"],
  "filterableAttributes": ["kind", "repo", "path", "topics", "severity", "pii_risk", "ts", "language",
                           "ext.decision.status", "ext.law_violation.law_id"],
  "sortableAttributes":   ["ts"],
  "displayedAttributes":  ["*"],
  "rankingRules": [
    "words",
    "typo",
    "proximity",
    "attribute",
    "sort",
    "exactness",
    "ts:desc"                                    // newer > older, all else equal
  ],
  "stopWords": ["a","an","the","of","in","to","and","or"],
  "synonyms": {
    "refactor": ["rewrite", "refactoring"],
    "bug":      ["issue", "defect"],
    "law":      ["policy", "rule"]
  },
  "typoTolerance": {
    "enabled": true,
    "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 8 }
  }
}
```

Synonyms and stop-words live in `cortex-workers/meilisearch/settings.v<N>.json`, versioned. Changing them triggers a **settings bump** but not a reindex — Meilisearch re-ranks on the fly.

### Meilisearch client

```rust
pub struct MeiliClient {
    endpoint: Url,                           // http://localhost:7700
    api_key: String,
    http: reqwest::Client,
    retry: RetryPolicy,                      // 3 attempts, exp backoff 100/400/1600 ms
}

impl MeiliClient {
    async fn ensure_index(&self, name: &str, settings: &IndexSettings) -> Result<()>;
    async fn upsert_documents(&self, index: &str, docs: &[Document]) -> Result<TaskUid>;
    async fn wait_task(&self, task: TaskUid, timeout: Duration) -> Result<TaskStatus>;
}
```

- **Async task model:** Meilisearch upserts are async (returns a `taskUid`). For low-volume live traffic the worker **fire-and-forgets**; for bootstrap it **waits** to detect failures early (flag `CORTEX_FULLTEXT_AWAIT_TASK=1`).
- **Batch size:** 1 000 documents per upsert call. Meilisearch is throughput-friendly; the bottleneck is the HTTP round-trip, not document parsing.
- **Idempotency:** `doc_id`-keyed; dupes are overwrites (same bytes → same result).

### Per-kind document builder

`cortex-workers/src/fulltext/doc_builder.rs` has one function per event family:

```rust
fn build_tool_call_doc(e: &EnrichedEvent) -> Document { ... }
fn build_decision_doc(e: &EnrichedEvent) -> Document { ... }
fn build_turn_doc(e: &EnrichedEvent) -> Document { ... }
// ...
```

Each builder is pure (no I/O) and trivially unit-testable. The dispatcher is an exhaustive match on `kind` — adding a new kind is a compile error until a builder exists.

### Body selection (what goes into `body`)

The `body` field is the primary searchable text. Selection priority:

1. If the event has a `classifier.summary` **and** raw payload > 4 KB → use the summary (same rule as spec 06 — keeps cost bounded).
2. Otherwise, use the **redacted raw payload text** (for tool_call: `input.command` or `input.text`; for turn: the user prompt; for artifact: the chunk text from spec 06).
3. If neither is present, skip the event with `counter: fulltext.skipped_empty`.

The raw payload is **never** indexed without redaction.

### Worker concurrency

```
Synap consumer ──▶ doc builder pool ──▶ Meili upsert pool ──▶ publisher (cortex.events.fulltext_indexed)
```

**Knobs (env):**
- `CORTEX_FULLTEXT_WORKERS=4`
- `CORTEX_FULLTEXT_BATCH=1000`
- `CORTEX_FULLTEXT_FLUSH_MS=1000`
- `CORTEX_FULLTEXT_AWAIT_TASK=0` (1 during bootstrap)

Backpressure: Meilisearch returns 503 when overloaded; the worker backs off and pauses the consumer after sustained 503s for >30 s.

### Failure modes

| Failure                           | Handling                                                                 |
|-----------------------------------|--------------------------------------------------------------------------|
| Meili 503 / rate limit            | Retry with backoff; eventually pause consumer                            |
| Meili 400 (schema error, bad id)  | Fail event → `cortex.events.invalid` with cause                          |
| Task failed (`wait_task` == failed) | Record `fulltext.task_failures`; dead-letter the batch                  |
| Settings incompatible (new rankingRules conflict) | Fail fast at startup with clear message                  |
| Empty body + no summary           | Skip; counter `fulltext.skipped_empty`                                   |
| Oversize single document (>10 MB) | Truncate `body` to 10 MB, set `truncated=true`; counter increments        |

### Observability

```
cortex.fulltext.documents.total       counter, labels: index
cortex.fulltext.batch.size            histogram
cortex.fulltext.upsert.latency_ms     histogram, labels: index
cortex.fulltext.dedup.hits            counter
cortex.fulltext.task_failures         counter, labels: reason
cortex.fulltext.errors                counter, labels: status
cortex.fulltext.skipped_empty         counter
cortex.fulltext.truncated             counter
cortex.fulltext.backpressure.active   gauge
```

## Acceptance criteria

- [ ] Startup against an empty Meilisearch creates all indexes with the declared settings; idempotent on re-run.
- [ ] 10 000-event synthetic stream lands in the correct indexes (per-kind split); counts match by kind.
- [ ] Idempotency: replaying the same 10 000 events results in zero new documents (dedupe hits == documents).
- [ ] Bootstrap identity: a doc_id like `bootstrap:Vectorizer:src/lib.rs:<hash>` remains stable across two bootstrap runs.
- [ ] Body fallback: a 20 KB payload with a classifier summary uses the summary as `body`; `truncated` is false.
- [ ] Body fallback: a 20 KB payload **without** a summary truncates at 10 MB-boundary isn't reached but the raw path is still stored (`summary` null, `body` contains full redacted text).
- [ ] Typo tolerance: query "refator" returns documents containing "refactor" within the first 10 hits on the `cortex-code` index with a 1 000-doc sample.
- [ ] Synonyms: query "bug" returns docs mentioning "defect" in ranked results.
- [ ] Filterable: `filter: 'repo = Vectorizer AND severity = critical'` returns only matching docs.
- [ ] Sortable: `sort: 'ts:desc'` orders results newest-first on identical-score ties.
- [ ] Settings bump (v1 → v2) updates ranking rules without reindexing; counter `fulltext.settings_bump` increments.
- [ ] Meilisearch 503 soak: 1-minute rate-limit, zero event loss, drains after the storm.
- [ ] Schema drift: pre-existing incompatible settings fail at startup with a clear message.
- [ ] Telemetry non-zero after soak; P95 upsert latency < 300 ms on dev stack with local Meilisearch.

## Decisions

1. **Per-kind indexes, not one mega-index.** Matches embedder collections (spec 06) and lets us tune ranking rules per family (code ranks differ from decisions).
2. **Meilisearch, not Lexum, in v1.** Decision ratified in architecture §12; Lexum is not production-ready. Swap is a client-side migration (write a `LexumClient` implementing the same trait) — no schema changes upstream.
3. **Fire-and-forget in live mode, await in bootstrap.** Live traffic can tolerate async task processing; bootstrap needs fail-fast to avoid silent corpus corruption.
4. **Synonyms + stop-words versioned.** They're part of the retrieval contract; changing them must be auditable.
5. **Body selection is deterministic and explicit.** No ML-driven "choose the best text" — we pick summary OR raw, never both, never a heuristic blend.
6. **Truncate at 10 MB, don't reject.** Losing the tail of a giant payload is better than losing the whole document for retrieval.

## Open questions

1. **Cross-index search.** Do we expose a multi-index search endpoint in Cortex, or does the orchestrator (spec 11) issue N parallel single-index searches? Leaning orchestrator-side federation for control over ranking; revisit when spec 11 benchmarks land.
2. **Language-aware tokenization for code.** Meilisearch's tokenizer is natural-language tuned. Snake_case symbols may underperform. Custom tokenizer (or pre-processing pass) — defer until retrieval quality pass in Phase 2.

## References

- Architecture §5.2, §5.3 (retrieval lanes), §12 OQ on Lexum.
- Spec 01 — Event schema.
- Spec 02 — Storage layout (Meilisearch instance, index namespaces).
- Spec 04 — Cortex Core.
- Spec 05 — Classifier (`summary` usage).
- Spec 06 — Embedder (parallel consumer; same collection boundaries).
- Spec 07 — Graph writer (parallel consumer).
- Spec 11 — Query API (reads these indexes).
- Meilisearch docs: https://www.meilisearch.com/docs — settings API, typo tolerance, filterable/sortable attributes.
