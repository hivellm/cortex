# 06 — Embedder (chunking + Vectorizer client)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 01, 02

## Goal

Consume enriched events, chunk their payloads deterministically (symbol-level for code via Tree-sitter, section-level for docs, fixed-token otherwise), produce embeddings through the **Vectorizer** service (BM25 + dense hybrid, per-`kind` collections), and write vectors with a stable chunk identity so re-runs are idempotent. The embedder does **not** own the vector index — Vectorizer does. This spec is about the client: chunking, batching, deduping, writing.

## Scope

**In:**
- Worker consuming `cortex.events.enriched`.
- Chunker trait + three default implementations (code / doc / fallback).
- Tree-sitter language bindings for the Phase-1 language set.
- Chunk identity (`chunk_id`) and content-hash-based dedup.
- Vectorizer client (HTTP/gRPC) with retry, backoff, batch write.
- Per-`kind` collection routing + schema registration.
- Summary substitution for payloads >4 KB (uses classifier `summary`).
- Telemetry.

**Out:**
- Vectorizer infrastructure (separate service, owned by the Vectorizer team).
- Embedding model choice beyond "whatever Vectorizer exposes as the default per collection" — see Open questions.
- Cross-chunk relationships (edges live in Nexus, spec 07).
- Query-side logic (spec 11).

## Inputs / Outputs

### Trait

```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed_batch(&self, events: &[EnrichedEvent]) -> Result<EmbedReport>;
}

pub struct EnrichedEvent {
    pub event_id: String,
    pub kind: Kind,
    pub content_hash: String,
    pub redacted_payload: serde_json::Value,
    pub classifier: ClassifierOutput,        // from spec 05
    pub context_repo: Option<String>,
    pub context_path: Option<String>,
}

pub struct EmbedReport {
    pub chunks_written: u32,
    pub chunks_deduped: u32,                 // already in Vectorizer with same content_hash
    pub chunks_skipped_empty: u32,
    pub by_collection: BTreeMap<String, u32>,
    pub latency_ms: u32,
    pub errors: Vec<EmbedError>,
}
```

### Chunker trait

```rust
pub trait Chunker: Send + Sync {
    fn chunk(&self, event: &EnrichedEvent) -> Result<Vec<Chunk>>;
}

pub struct Chunk {
    pub chunk_id: String,                    // deterministic; see below
    pub parent_event_id: String,
    pub parent_content_hash: String,
    pub chunk_content_hash: String,          // hash of this chunk's text
    pub collection: String,                  // per-kind; see below
    pub text: String,                        // what gets embedded
    pub metadata: ChunkMetadata,
}

pub struct ChunkMetadata {
    pub kind: Kind,
    pub topics: Vec<String>,
    pub severity: Severity,
    pub repo: Option<String>,
    pub path: Option<String>,
    pub symbol: Option<String>,              // fn/struct/class name, code chunks
    pub byte_range: Option<(u32, u32)>,
    pub language: Option<String>,
    pub source: ChunkSource,                 // code | doc | summary | fallback_window
    pub prompt_version: Option<String>,      // if text came from classifier summary
}
```

### Chunk identity

```
chunk_id = ulid_from_hash(parent_event_id || ':' || chunk_ordinal || ':' || chunk_content_hash)
```

- Deterministic for the same input → re-runs are idempotent.
- `chunk_ordinal` is the 0-based index within the event so a single event always maps to a stable sequence.
- Re-chunking the same event with the same chunker version produces the exact same `chunk_id` set, so Vectorizer sees no-op writes.

### Collections (per-kind)

| Kind                  | Collection              | Why separate                                                |
|-----------------------|-------------------------|-------------------------------------------------------------|
| `tool_call.*` (code)  | `cortex-code`           | Symbol-level; favors small HNSW `ef_search`                 |
| `artifact.doc`        | `cortex-docs`           | Larger chunks; favors `ef_search` tuned for recall          |
| `decision`            | `cortex-decisions`      | Small N, very high weight; favors recall > latency          |
| `turn.*`              | `cortex-turns`          | High write throughput; favors insert rate                   |
| `law` / `law_violation` | `cortex-governance`   | Small, long-lived; per-deployment                            |
| everything else       | `cortex-misc`           | Catch-all                                                   |

Collection names are prefixed with the deployment namespace (default `cortex-`) so a single Vectorizer instance can host multiple deployments.

## Design

### Chunkers

1. **CodeChunker (Tree-sitter).**
   - Grammars loaded once at startup. Phase-1 set: Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, Markdown, JSON, YAML, TOML.
   - Strategy: top-level declarations only (fn/struct/impl/trait/class/interface/module). Nested items do **not** emit their own chunk — they live inside the parent.
   - Oversize top-level declaration (>8 KB after whitespace normalization): fall back to sliding-window split (window=512 tokens, stride=128).
   - Emits `symbol`, `byte_range`, `language` in metadata.

2. **DocChunker (Markdown).**
   - Split on H1/H2/H3 boundaries; merge tiny sections (<256 chars) with the next one.
   - Strips code fences longer than 40 lines (re-emitted as sibling code chunks).
   - Preserves the section path as `symbol` (`"## Architecture > ### Data flow"`).

3. **FallbackChunker (fixed window).**
   - 512-token window, 128-token stride.
   - Used when language is unknown or Tree-sitter grammar is missing.

### Summary substitution

For any chunk whose **raw byte size > 4 KB**, the embedder substitutes the classifier `summary` as the embedded text, *and* emits the raw chunk as a separate record with `source=raw_oversize` that stays in Vectorizer metadata but is **not re-embedded** (it's only fetchable as context). This keeps embedding inputs small while preserving the full content for retrieval.

Rule table:

| Raw size | Classifier summary | Action                                                             |
|----------|--------------------|--------------------------------------------------------------------|
| ≤4 KB    | ignored            | Embed raw text                                                      |
| >4 KB    | present            | Embed `summary`; store raw in Vectorizer metadata (`full_text`)    |
| >4 KB    | missing            | **Error** — route event to `cortex.events.invalid` with cause      |

### Vectorizer client

```rust
pub struct VectorizerClient {
    base_url: Url,                           // e.g. http://localhost:15002
    api_key: Option<String>,
    http: reqwest::Client,
    retry: RetryPolicy,                      // 3 attempts, exp backoff 100ms/400ms/1600ms
}

impl VectorizerClient {
    async fn ensure_collection(&self, name: &str, schema: &CollectionSchema) -> Result<()>;
    async fn upsert_chunks(&self, collection: &str, chunks: &[Chunk]) -> Result<UpsertReport>;
    async fn exists(&self, collection: &str, chunk_ids: &[String]) -> Result<BitSet>;
}
```

- **Transport:** HTTP POST `/v1/collections/{name}/upsert` with JSON body. gRPC path is an optimization flag (see Decisions §3).
- **Batch size:** 64 chunks per upsert call. Vectorizer handles embedding internally (the client sends `text` + metadata, never precomputed vectors).
- **Idempotency:** driven by `chunk_id`. Vectorizer upsert is key-based, so re-sends are no-ops at the vector store level.
- **Dedup pre-check (optional):** before sending a large batch, the worker calls `/v1/collections/{name}/exists?ids=...` and filters. Reduces Vectorizer embed-call volume by ~30% during bootstrap re-runs.

### Worker concurrency

```
Synap consumer ──▶ chunker pool ──▶ Vectorizer client pool ──▶ publisher (cortex.events.embedded)
```

**Knobs (env):**
- `CORTEX_EMBEDDER_WORKERS=6` (default; raise for bootstrap)
- `CORTEX_EMBEDDER_CHUNKER_CONCURRENCY=4` (chunker threads per worker)
- `CORTEX_EMBEDDER_UPSERT_BATCH=64`
- `CORTEX_EMBEDDER_MAX_RETRY=3`

Backpressure: if Vectorizer 429s sustained for >30 s, the worker pauses its Synap consumer (cooperative flow control) rather than buffering unbounded.

### Collection schema registration

On worker startup, each known collection is `ensure_collection`'d:

```jsonc
{
  "name": "cortex-code",
  "vector": { "dim": 768, "metric": "cosine" },   // Vectorizer picks the model
  "hybrid": { "bm25": true, "dense": true },
  "metadata_index": ["kind", "topics", "repo", "path", "language", "severity"]
}
```

If a collection exists with incompatible schema, the worker **fails fast** — schema migrations are a human-in-the-loop operation (see Decisions §5).

### Failure modes

| Failure                                    | Handling                                                            |
|--------------------------------------------|---------------------------------------------------------------------|
| Tree-sitter parse error                    | Log, fall back to `FallbackChunker`, tag chunk source `fallback_window` |
| Unknown language                           | `FallbackChunker`                                                    |
| Oversize chunk with no classifier summary  | Event → `cortex.events.invalid` with cause `oversize_without_summary` |
| Vectorizer 429/5xx                         | Retry (exp backoff), then pause consumer; no dead-letter            |
| Vectorizer 400 (schema mismatch)           | Fail fast; human must resolve schema                                 |
| Empty payload after redaction              | Skip event; counter `embedder.skipped_empty`                        |

### Observability

```
cortex.embedder.chunks.total          counter, labels: source (code|doc|summary|fallback_window)
cortex.embedder.chunks.bytes          histogram
cortex.embedder.upsert.latency_ms     histogram, labels: collection
cortex.embedder.upsert.batch_size     histogram
cortex.embedder.dedup.hits            counter
cortex.embedder.vectorizer.errors     counter, labels: status
cortex.embedder.backpressure.active   gauge (0|1)
cortex.embedder.oversize_without_summary counter
```

Span per event: `event_id`, `chunks.emitted`, `chunks.deduped`, `collections.touched`, `latency.chunk_ms`, `latency.upsert_ms`.

## Acceptance criteria

- [ ] Code chunker produces one chunk per top-level declaration on a 1 000-LOC Rust sample; `symbol` and `byte_range` set correctly; round-trip content matches original.
- [ ] Oversize declaration (>8 KB) falls back to windowed chunks, each marked `fallback_window`, and reconstructs losslessly on `JOIN ORDER BY chunk_ordinal`.
- [ ] Doc chunker splits `README.md` on section boundaries; section path stored in `symbol`.
- [ ] Summary substitution: a synthetic 20 KB payload with `classifier.summary` set produces one embedded chunk (the summary) and one `source=raw_oversize` record with `full_text`; no embedding call for the raw record.
- [ ] Oversize without summary routes to `cortex.events.invalid`, counter increments, no partial write.
- [ ] Per-`kind` routing: 5 events of mixed kinds land in the correct collections (verified via Vectorizer `/v1/collections/.../query`).
- [ ] Idempotency: re-running the same event produces zero new chunks in Vectorizer (`chunks_deduped == chunks_emitted`).
- [ ] Vectorizer 429 soak: 10 000-event burst against a rate-limited Vectorizer backs off and eventually drains with zero loss.
- [ ] Schema drift: bringing up against a pre-existing incompatible collection fails the worker at startup with a clear message (no silent corruption).
- [ ] Tree-sitter missing grammar: Elixir file is processed via `FallbackChunker`, tagged accordingly, no crash.
- [ ] Telemetry counters non-zero after soak; P95 upsert latency < 400 ms on dev machine with local Vectorizer.

## Decisions

1. **Client, not model.** Cortex does not run embedding models. Vectorizer owns the model (FastEmbed / MiniLM by default). If we need a different model per collection, we request it from Vectorizer, we do not ship our own runtime.
2. **Symbol-level chunking for code.** Precision matters more than recall for pre-thinking context — smaller, well-scoped chunks beat large windows. Tree-sitter is the right tool; we pay the grammar-maintenance cost.
3. **HTTP first, gRPC later.** HTTP JSON is sufficient for Phase-1 throughput; gRPC is an optimization flag (`CORTEX_EMBEDDER_TRANSPORT=grpc`) when/if we need it.
4. **No precomputed vectors on the wire.** We always send text to Vectorizer and let it embed. Keeps the boundary clean and lets Vectorizer swap models without Cortex redeploys.
5. **Schema drift is fail-fast.** Silent migrations corrupt retrieval. A human must run `cortex embedder migrate <collection>` (tooling lives in `cortex-cli`, out of scope here).
6. **Raw-oversize text stays in Vectorizer metadata, not on disk.** Vectorizer is the system of record for chunk text in v1. Parquet archive (spec 02) is the durable copy; Vectorizer is the queryable copy.

## Open questions

1. **Per-collection model override.** Should `cortex-decisions` use a bigger model (e.g., MiniLM-L12) for better semantic fidelity, given the low volume? Defer to first retrieval-quality pass (Phase 2).
2. **Chunk overlap for docs.** Current plan: no overlap (section boundaries are semantically clean). If retrieval evaluation shows boundary-loss, revisit with a 1-sentence overlap rule.

## References

- Architecture §5.2 (processing pipeline), §6 (bootstrap chunking choices).
- Spec 01 — Event schema (envelope / `content_hash`).
- Spec 02 — Storage layout (Vectorizer collections, Parquet archive).
- Spec 04 — Cortex Core (what produces `EnrichedEvent`).
- Spec 05 — Classifier (`summary` field for oversize payloads).
- Spec 07 — Graph writer (sibling consumer; receives same enriched events).
- Spec 11 — Query API (consumes these collections).
- Vectorizer docs: `e:/HiveLLM/Vectorizer/README.md`, hybrid pipeline section.
- Tree-sitter: https://tree-sitter.github.io
