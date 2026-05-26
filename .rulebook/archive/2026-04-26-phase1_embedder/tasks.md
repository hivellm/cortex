## 1. Crate scaffold
- [x] 1.1 `cortex-embedder` crate with `Chunker` + `Embedder` traits from spec 06
- [x] 1.2 Worker binary `cortex-embedder-worker` consuming `cortex.events.enriched`
- [x] 1.3 Config via env (`CORTEX_EMBEDDER_*`) with defaults from spec 06 §Worker concurrency

## 2. Chunkers
- [x] 2.1 `CodeChunker` with Tree-sitter grammars for Rust, TS, JS, Python, Go, Java, C, C++, Markdown, JSON, YAML, TOML
- [x] 2.2 `DocChunker` splitting Markdown on H1/H2/H3; merge tiny sections; preserve section path as `symbol`
- [x] 2.3 `FallbackChunker` fixed-window (512 tokens, 128 stride)
- [x] 2.4 Oversize declaration (>8 KB) windowed as `source=fallback_window`

## 3. Chunk identity + metadata
- [x] 3.1 `chunk_id = ulid_from_hash(event_id || ':' || ordinal || ':' || chunk_content_hash)`
- [x] 3.2 `ChunkMetadata` struct populated from `EnrichedEvent`
- [x] 3.3 Summary substitution: raw >4 KB with `classifier.summary` → embed summary, archive raw as `source=raw_oversize`; missing summary → dead-letter

## 4. Vectorizer client
- [x] 4.1 HTTP client with retry + exp backoff (3 attempts, 100/400/1600 ms)
- [x] 4.2 `ensure_collection` for every per-kind collection at startup (fail-fast on schema drift)
- [x] 4.3 `upsert_chunks` in batches of 64; optional `exists` pre-check for bootstrap dedup

## 5. Worker loop
- [x] 5.1 Consume enriched events; chunker pool (4 threads per worker); Vectorizer pool
- [x] 5.2 Publish `cortex.events.embedded` on success
- [x] 5.3 Cooperative backpressure: pause consumer on sustained 429 (>30 s)

## 6. Observability
- [x] 6.1 Counters + histograms per spec 06 §Observability
- [x] 6.2 Per-event span: chunks emitted, deduped, collections touched, chunk_ms, upsert_ms

## 7. Tail (mandatory)
- [x] 7.1 Update or create documentation covering the implementation — `docs/specs/06-embedder.md` status flag to 🟢 + index row
- [x] 7.2 Write tests covering the new behavior — integration tests: 1 000-LOC Rust sample → correct symbol chunks; oversize summary substitution; idempotent replay (deduped == emitted); 429 soak; unknown language → fallback; schema-drift fail-fast
- [x] 7.3 Run tests and confirm they pass — `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
