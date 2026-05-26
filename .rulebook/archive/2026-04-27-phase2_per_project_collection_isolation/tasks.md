## 1. Slug helper
- [x] 1.1 Add `slug_for_repo(repo_id: &str) -> String` to `crates/cortex-storage/src/names.rs`
- [x] 1.2 Unit tests cover empty, ASCII alpha, mixed case, accents, slashes, leading/trailing dashes

## 2. Embedder write path
- [x] 2.1 Change `routing::collection_for(kind, prefix)` to `collection_for(kind, prefix, repo_id)`
- [x] 2.2 Change `routing::collection_for_chunk(kind, source, prefix)` to take `repo_id`
- [x] 2.3 Update `chunker_code.rs`, `chunker_doc.rs`, `chunker_fallback.rs` to derive the slug from `event.context_repo` (fallback `unknown`)
- [x] 2.4 Update `embedder.rs` accounting comparisons (`collection_for` callers) to thread `repo_id`
- [x] 2.5 `vectorizer_client::ensure_collection` already runs per-event; new collections appear on first write
- [x] 2.6 Tests: `event with context_repo="Cortex"` lands in `cortex-cortex-{family}`; `context_repo=None` lands in `cortex-unknown-{family}`

## 3. Fulltext write path
- [x] 3.1 Change `routing::index_for(prefix, kind)` to take `repo_id`
- [x] 3.2 Update writer pipeline to pass `event.context_repo` through `index_for`
- [x] 3.3 Existing `meili_client::ensure_index` is per-event; lazy-creation pattern preserved
- [x] 3.4 Tests mirror the embedder set

## 4. Read path (orchestrator)
- [x] 4.1 `cortex-api::strategies` builds lane requests with `cortex-{slug}-{family}` derived from `req.scope.repo`
- [x] 4.2 Empty `req.scope.repo`: lane requests use `cortex-unknown-{family}` (zero hits — surfaces missing scope to caller)
- [x] 4.3 Tests: query with `scope.repo = "Vectorizer"` hits the per-repo collection name

## 5. Build + rollout
- [x] 5.1 `cargo check` clean across cortex-storage, cortex-embedder, cortex-fulltext, cortex-api
- [x] 5.2 `cargo test` green in each (~108 tests)
- [x] 5.3 `cargo build --release` for cortex-embedder-worker, cortex-fulltext-worker, cortex-api
- [x] 5.4 Replace `~/.cargo/bin/` binaries; cortex-api restarted on new binary; embedder/fulltext binaries swapped (workers respawn at user discretion)
- [x] 5.5 Smoke test: `POST /v1/query` with `scope.repo="Cortex"` returned `scope_resolved.repo = "Cortex"`; lanes ran clean (vector_ms / keyword_ms = 0 with empty in-memory lanes, as expected post-wipe). Routing output confirmed via the orchestrator's own lane-request construction; full end-to-end probe against live Vectorizer / Meili needs the workers running which is gated by user-triggered bootstrap

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update documentation: spec-06 § Collection naming, spec-08 § Index naming
- [x] 6.2 Write tests covering the new behavior
- [x] 6.3 Run tests and confirm they pass
