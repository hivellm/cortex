# 06 — Embedder (chunking + Vectorizer client)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 02

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

- **Primary id**: server-assigned UUID. Vectorizer's `POST /insert_texts`
  ignores any client-supplied `id` and returns a server-generated UUID
  per entry in the `BatchResponse`. The embedder treats this UUID as
  the canonical chunk id and carries it in `UpsertedChunk::server_id`
  for downstream consumers (graph writer, query API).
- **Dedup key** (client-side, metadata): a deterministic string stored
  as `metadata.dedup_key` on every vector:

  ```
  dedup_key = ulid_from_hash(parent_event_id || ':' || chunk_ordinal
                             || ':' || chunk_content_hash)
  ```

  The orchestrator does a `list_vectors`-paginated scan before upsert
  to collect the present `dedup_key` set for each target collection,
  filtering out already-embedded chunks. Re-runs produce the same
  `dedup_key` set → zero new upserts → idempotent re-runs.

### Collections (per-kind, per-repo)

| Kind                    | Collection family        | Why separate                                                |
|-------------------------|--------------------------|-------------------------------------------------------------|
| `tool_call.*` (code)    | `code`                   | Symbol-level; favors small HNSW `ef_search`                 |
| `artifact.doc`          | `docs`                   | Larger chunks; favors `ef_search` tuned for recall          |
| `decision`              | `decisions`              | Small N, very high weight; favors recall > latency          |
| `turn.*`                | `turns`                  | High write throughput; favors insert rate                   |
| `law` / `law_violation` | `governance`             | Small, long-lived; per-deployment                            |
| `knowledge` (phase10e)  | `knowledge`              | Single-tier; small + dense; reference material              |
| `learning` (phase10e)   | `learnings`              | Single-tier; high signal; preserved at full precision       |
| everything else         | `misc`                   | Catch-all                                                   |

#### Single-tier kinds (phase10e)

`knowledge` + `learning` are deliberately **single-tier** (no PQ
warm tier). The corpus is tiny (~60 entries / repo for the
canonical Hive workspace), dense (every entry was hand-curated),
and high-signal (each was written specifically because someone
made a mistake worth not repeating). Demoting to a PQ tier would
lose precision the agent needs when re-reading the entry
verbatim — the cost of keeping every vector in `fp32` is
trivial compared to the retrieval-quality hit. See
[spec 02 §Knowledge + Learnings corpus (phase10e)](./02-storage-layout.md#knowledge--learnings-corpus-phase10e)
for the cross-store contract.

The full collection name is `{prefix}-{repo_slug}-{family}` (default prefix `cortex`). Per-project isolation is mandatory: every event carries a `context.repo` from the upstream emitter, the routing layer slugifies it through `cortex_storage::names::slug_for_repo`, and each repo writes into its own collection — `cortex-cortex-docs`, `cortex-tml-code`, `cortex-vectorizer-turns`, etc. Events with no `context.repo` route to the `unknown` slug so the produced name is always well-formed; downstream queries can scope to a single repo (or fan out across slugs in a future revision).

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

### Embedding text projection (phase0)

Before a chunk is embedded, `chunker_fallback.rs::event_text()` converts the
`EnrichedEvent` payload into a natural-language string using the following
priority order:

1. **Classifier summary** — if `event.classifier.summary` is `Some(s)` and
   non-empty, `s` is used verbatim. Applies only when the classifier runs in
   LLM mode; in `Static` mode the summary is `None`.
2. **Per-kind NL projection** — deterministic rendering of the payload's most
   meaningful fields, matched on `event.kind`:

   | Kind | Projection |
   |------|-----------|
   | `turn` | `user_message` + `assistant_message`, joined with `\n` |
   | `tool_call` | `"{tool_name}({input_json})"` + `output.text` |
   | `agent_call` | `description` + `prompt` |
   | `memory` | `name` + `body` + `description` |
   | `decision` | `title` + `status` + `body` |
   | `analysis` | `question` |
   | `law` | `title` + `body` |
   | `law_violation` | `message` |
   | `artifact` | `context_path` + `body` |
   | `knowledge` | `title` + `body` |
   | `learning` | `title` + `body` |
   | `consolidation` | `summary_markdown` + `takeaways[]` joined with `\n` |
   | `topic_card` | `synthesis_markdown` |

3. **Legacy field scan** — falls back to the first non-empty string found in
   `content`, `text`, or `body` fields of the raw JSON payload.
4. **JSON fallback** — `serde_json::to_string_pretty(payload)` as a last resort.

**Re-index note:** the dedup key is derived from the chunk's content hash
(`ULID(SHA256(event_id:ordinal:chunk_content_hash))`). Changing the projected
text changes the content hash and therefore the dedup key — new NL-projected
vectors are written as new entries alongside any pre-existing JSON-projected
vectors. A clean re-index (purging stale vectors first) is required for a pure
NL corpus.

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
    base_url: Url,                           // e.g. http://localhost:17002
    api_key: Option<String>,
    http: reqwest::Client,
    retry: RetryPolicy,                      // 3 attempts, exp backoff 100ms/400ms/1600ms
}

impl VectorizerClient {
    async fn ensure_collection(&self, name: &str, schema: &CollectionSchema) -> Result<()>;
    async fn upsert_chunks(&self, collection: &str, chunks: &[Chunk]) -> Result<UpsertReport>;
    async fn exists_by_dedup_key(&self, collection: &str, dedup_keys: &[String]) -> Result<BTreeSet<String>>;
}

pub struct UpsertReport {
    pub written: u32,
    pub deduped: u32,
    pub new_entries: Vec<UpsertedChunk>,   // dedup_key → server-assigned UUID
}

pub struct UpsertedChunk {
    pub dedup_key: String,
    pub server_id: String,                 // opaque primary id (UUID on the v3 server)
}
```

- **Transport:** HTTP POST `/insert_texts` via the SDK (v3.0.3+); `exists_by_dedup_key` paginates `GET /collections/{c}/vectors` until the SDK grows a `list_vectors` surface.
- **Batch size:** 64 chunks per upsert call. Vectorizer handles embedding internally (the client sends `text` + metadata, never precomputed vectors).
- **Idempotency:** driven by `metadata.dedup_key`. The orchestrator pre-scans the target collection's `dedup_key` set and filters already-embedded chunks before calling `upsert_chunks` — the server itself does no dedup.
- **Dedup pre-check (mandatory):** every `embed_batch` call runs the scan so re-runs produce zero new upserts. Reduces Vectorizer embed-call volume to zero during bootstrap re-runs.

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

## Read path

The write path above is consumed at query time by `cortex-api`'s
vector lane (spec 11 §Lane traits). The lane translates the
orchestrator's `VectorRequest { collection, query, k, scope }` into
the Vectorizer SDK's `search_vectors(collection, query, limit,
threshold)` and projects each `SearchResult` into the `LaneHit`
shape the fusion stage expects.

### Live lane: `VectorizerLane`

Lives at [crates/cortex-api/src/vectorizer_lane.rs](../../crates/cortex-api/src/vectorizer_lane.rs).
Selected at daemon startup when `CORTEX_VECTORIZER_URL` (or
`VECTORIZER_URL`) is set **and** the SDK's `health_check` succeeds.
On any failure (env unset, server unreachable, build error,
unauthorised) the daemon falls back to `MemoryVectorLane` so
cold-stack dev keeps working — the failure is logged at WARN with
the URL + reason.

Auth selection mirrors `cortex-embedder-worker`'s boot flow:

1. **Explicit JWT / API key** — `CORTEX_VECTORIZER_API_KEY` or
   `VECTORIZER_API_KEY` wins when set; passed through as the SDK's
   `api_key` (the 3.0.3 HTTP transport sniffs the JWT shape and
   sends it as `Authorization: Bearer …`).
2. **Username + password** — when both `CORTEX_VECTORIZER_USER` /
   `CORTEX_EMBEDDER_VECTORIZER_USER` and the matching `*_PASSWORD`
   are set, the lane runs `POST /auth/login` once at boot via the
   SDK's `login()` and uses the minted JWT.
3. **No auth** — falls through to a no-credential client (Vectorizer
   running with `auth.enabled: false`).

### Search request shape

```rust
client.search_vectors(
    /* collection: */ &req.collection, // cortex-{slug}-{family}
    /* query:      */ &req.query,
    /* limit:      */ Some(req.k),
    /* threshold:  */ None,
).await
```

The SDK serialises this into `POST /collections/{uid}/search/text`
with `{ query, limit }`. `score_threshold` is left server-default
because the orchestrator's RRF fusion already applies a normalisation
pass over the lane scores — pre-filtering here would interact poorly
with the fusion stage.

### Hit projection (Vectorizer → `LaneHit`)

| LaneHit field        | Vectorizer source                                                |
|----------------------|------------------------------------------------------------------|
| `doc_id`             | `vec|{collection}|{result.id}`                                  |
| `text`               | `result.content` → `metadata.summary` → `metadata.title` → `metadata.body` (first non-empty wins) |
| `repo`               | `metadata.repo`                                                  |
| `path`               | `metadata.path`                                                  |
| `symbol`             | `metadata.kind`                                                  |
| `content_hash`       | `metadata.content_hash`                                          |
| `score`              | `result.score` (the SDK's normalised similarity, `[0, 1]`)       |
| `ts`                 | `metadata.ts` (defaults to `0` when absent)                      |
| `severity`           | `metadata.severity`                                              |
| `extras["source"]`   | `"vector"` (constant — source-attribution invariant)             |
| `extras["collection"]` | `req.collection` (so debug surfaces can correlate hits to the per-project collection name) |

Empty `text` is acceptable: some embedded chunks carry no inline
content (the chunker stored the body elsewhere). The fusion stage
still ranks the hit on score; the snippet column simply lacks a
preview, which is honest.

### Failure handling

- `2xx` — parse `results[]` and project each into a `LaneHit`.
- 404 / "not found" (collection not yet materialised) — return
  `Vec::new()`. Per-project collections are created lazily by the
  worker on first upsert; an empty result is the legitimate case,
  not an error.
- Other SDK errors — `LaneError::Transport(detail)`. The
  orchestrator's fail-open policy turns this into an empty hit set
  plus a `debug.errors["vector"]` entry; the response stays HTTP 200.

### Source-attribution invariant

Every hit produced by the vector lane MUST carry
`extras["source"] = "vector"`. The orchestrator's `lane_label()`
([crates/cortex-api/src/orchestrator.rs](../../crates/cortex-api/src/orchestrator.rs))
falls back to `"vector"` when missing — so a missing label happens to
work for the vector lane today, but stamping explicitly keeps the
keyword/vector lanes symmetric and prevents regressions if the
default ever changes. The keyword lane has the matching invariant
(`"keyword"`) checked by a `debug_assert!` in `Orchestrator::run`.

### Configuration

| Env var                                 | Default                  | Purpose                                                  |
|-----------------------------------------|--------------------------|----------------------------------------------------------|
| `CORTEX_VECTORIZER_URL`                 | (unset, falls back to `VECTORIZER_URL`) | Live lane base URL. Unset → MemoryVectorLane fallback. |
| `CORTEX_VECTORIZER_API_KEY`             | (unset, falls back to `VECTORIZER_API_KEY`) | JWT / API key (auth path 1).                            |
| `CORTEX_VECTORIZER_USER`                | (unset, falls back to `CORTEX_EMBEDDER_VECTORIZER_USER`) | Username for `/auth/login` (auth path 2).               |
| `CORTEX_VECTORIZER_PASSWORD`            | (unset, falls back to `CORTEX_EMBEDDER_VECTORIZER_PASSWORD`) | Password for `/auth/login` (auth path 2).               |

### Acceptance for the read path

- [ ] Two `/v1/query` calls with distinct `query` strings against the
      same collection return distinct `results.snippets` sets when
      semantically different documents exist.
- [ ] `debug.lanes.vector_ms` is non-zero on every query (the
      regression had `vector_ms = 0` because the `MemoryVectorLane`
      double never executed).
- [ ] Vectorizer down → response is HTTP 200,
      `debug.errors["vector"]` populated, `results` may be empty
      (fail-open).
- [ ] Snippet `source` column is `"vector"` for every hit produced
      by the vector lane.
- [ ] `/auth/login` is run **once** at boot when username + password
      are set; subsequent requests carry the minted JWT, never
      re-authenticate per request.

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
7. **Server-assigned UUIDs, client-side dedup keys.** The server rejects client-supplied primary ids (`hivehub/vectorizer:3.0.x` reassigns a UUID per stored vector regardless of the caller's `id`). Instead of forking the server or faking ids on our side, we adopt the server's UUID as canonical (surfaced through `UpsertedChunk::server_id` / `EmbedReport::new_records`) and carry our deterministic identifier as `metadata.dedup_key`. Idempotency is enforced by a pre-upsert scan of `metadata.dedup_key` rather than by the key-based upsert the original spec assumed. Downstream consumers that need to reference a stored vector join on the server-assigned UUID.

## Shared Synap-worker scaffolding (phase14h)

The embedder, fulltext indexer, graph writer, and classifier all
implement the same `SynapWorker` trait shipped in
`cortex_workers::synap_worker`. The shared module owns the loop
shape (back-off, supervisor, idle sleep, graceful shutdown, pool
join) so a fix in one place lands on all four workers at once.

Trait surface (`cortex_workers::synap_worker::SynapWorker`):

- `worker_name() -> &'static str` — stable label (`embedder` /
  `fulltext` / `graph` / `classifier`) used as the metric label
  and log target.
- `pool_size() -> usize` — number of `run_forever` copies the
  shared runtime spawns.
- `async fn run_once() -> Result<usize>` — one iteration; returns
  the number of envelopes handled so the runtime can decide
  whether to idle-sleep.
- `idle_duration() / backpressure_sleep() / error_backoff()` —
  per-worker sleep tunables (defaults: 100ms / 5s / 500ms).
- `backpressure() -> BackpressureGate` — `Active` / `Paused`;
  paused gates short-circuit `run_once` so the worker parks the
  loop without burning the Vectorizer / Nexus retry budget.
- `max_consume_errors() -> u32` — supervisor threshold; `0`
  disables the exit. Classifier sets this to the legacy
  `CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS` value, embedder /
  fulltext / graph default to `0` (back off forever).
- `on_run_once_ok / on_run_once_err / on_run_once_success_reset /
  on_backpressure_pause` — hooks the worker uses to bump its
  domain metrics (`record_jobs_processed`, `incr_errors`, the
  classifier `consume_errors_consecutive` mirror, etc.).

Driver surface:

- `run_forever(worker, shutdown) -> Result<(), RunError>` — single
  loop copy.
- `run_pool(worker, shutdown) -> Result<(), RunError>` — spawns
  `pool_size` copies onto the current Tokio runtime and joins
  them.

`RunError::Supervisor` carries the worker name, consecutive count,
threshold, and last error message — the bin's `main` propagates
the non-zero exit so docker `restart: unless-stopped` recovers
the container fresh (the phase11s contract is preserved).

Central telemetry (`cortex_workers::synap_worker::metrics::WorkerMetrics`):

- `cortex_synap_worker_lag{worker}` — last observed lag gauge,
  rendered in the Prometheus body via `WorkerMetrics::render_prom`.
- `cortex_synap_worker_dead_letter_total{worker, reason}` — counter
  family keyed by the `DeadLetterReason` taxonomy
  (`deserialize_failed`, `permanent_handler_error`,
  `retry_budget_exhausted`, `publish_failed`,
  `missing_required_field`). New reasons require a code change so
  the doctor's per-reason rendering stays stable.

Cursor checkpointing (`cortex_workers::synap_worker::checkpoint::CursorCheckpoint`)
re-uses the phase13b `producer_checkpoints` primitive. The
namespace is `synap_consumer:{worker_name}` and the scope is the
Synap room; `last_event_id` carries the offset as a stringified
`u64`. Workers seed their `OffsetTracker` from
`resume_offset(room)` on boot so kill-resume does not rewind to
offset `0`.

Operator surface: `cortex-ops doctor-synap-workers` probes each
worker's `/healthz`, parses the `state` / `last_consume_ts_ms` /
`consume_errors_consecutive` / `synap_worker_lag` extras, and
prints a per-worker table (or JSON via `--json`). Exit code `2`
when any worker reports non-`ok` state or a fetch failure.

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
