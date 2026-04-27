## 1. Slug helper
- [ ] 1.1 Add `slug_for_repo(repo_id: &str) -> String` to `crates/cortex-storage/src/names.rs`
- [ ] 1.2 Unit tests cover empty, ASCII alpha, mixed case, accents, slashes, leading/trailing dashes

## 2. Embedder write path
- [ ] 2.1 Change `routing::collection_for(kind, prefix)` to `collection_for(kind, prefix, repo_slug)`
- [ ] 2.2 Change `routing::collection_for_chunk(kind, source, prefix)` to take `repo_slug`
- [ ] 2.3 Update `chunker_code.rs`, `chunker_doc.rs`, `chunker_fallback.rs` to derive the slug from `event.context_repo` (fallback `"unknown"`)
- [ ] 2.4 Update `embedder.rs` accounting comparisons (`collection_for` callers at lines ~404 / ~474) to thread `repo_slug`
- [ ] 2.5 `vectorizer_client::ensure_collection` becomes lazy per-event when the collection is absent
- [ ] 2.6 Tests: `event with context_repo="Cortex"` lands in `cortex-cortex-{family}`; `context_repo=None` lands in `cortex-unknown-{family}`

## 3. Fulltext write path
- [ ] 3.1 Change `routing::index_for(prefix, kind)` to take `repo_slug`
- [ ] 3.2 Update writer pipeline to pass repo_slug from event
- [ ] 3.3 `meili_client::ensure_index` becomes lazy per-event
- [ ] 3.4 Tests mirror the embedder set

## 4. Read path (orchestrator)
- [ ] 4.1 `cortex-api::strategies` builds lane requests with `cortex-{slug}-{family}` derived from `req.scope.repos[0]`
- [ ] 4.2 Empty scope.repos: lane requests use `cortex-unknown-{family}` (will return zero hits — surfaces missing scope to caller)
- [ ] 4.3 Tests: query with `scope.repos = ["Cortex"]` hits the per-repo collection name

## 5. Build + rollout
- [ ] 5.1 `cargo check` clean across cortex-storage, cortex-embedder, cortex-fulltext, cortex-api
- [ ] 5.2 `cargo test` green in each
- [ ] 5.3 `cargo build --release` for cortex-embedder-worker, cortex-fulltext-worker, cortex-api
- [ ] 5.4 Replace `~/.cargo/bin/` binaries; respawn workers
- [ ] 5.5 Smoke test: drop a tiny envelope mentioning `Cortex` repo; confirm new collection `cortex-cortex-{family}` appears in Vectorizer / Meili

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update documentation: spec-06 § Collection naming, spec-08 § Index naming, spec-11 § Lane request shape
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
