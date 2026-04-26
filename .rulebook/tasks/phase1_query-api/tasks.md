## 1. Crate scaffold
- [x] 1.1 `cortex-api` crate (Axum + MCP tool binding)
- [x] 1.2 `POST /v1/query` with the spec-11 request/response types (`Intent`, `Scope`, `IncludeField`, `QueryRequest`, `QueryResponse` plus the per-overlay structs)
- [x] 1.3 MCP tool `cortex.query` exposing the same schema via `cortex_api::tool_descriptor` + `cortex_api::mcp_invoke`

## 2. Strategies
- [x] 2.1 Intent → execution-plan mapper for `pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `free_search`
- [x] 2.2 Scope canonicalisation echoed back via `scope_resolved`; CLI/MCP shape validates against the typed enum so unknown intents reject at deserialisation
- [x] 2.3 Per-intent lane + overlay set defined in `strategies::build_plan` with the spec-11 default sub-budget split

## 3. Lane clients
- [x] 3.1 `VectorLane` trait with KNN top-k semantics + `MemoryVectorLane` test double
- [x] 3.2 `KeywordLane` trait with filterable + sort-by-`ts` semantics + `MemoryKeywordLane` test double
- [x] 3.3 `GraphLane` trait with parametrised read-only Cypher + `MemoryGraphLane` test double; live Vectorizer / Meili / Nexus wiring drops in behind the same traits

## 4. Orchestrator
- [x] 4.1 Parallel fan-out through `tokio::join!` with the 40 / 40 / 20 sub-budget split (per-strategy override for `similar_problems` + `law_check`)
- [x] 4.2 RRF fusion (`score = Σ 1 / (60 + rank_lane)`) with deterministic tie-breaks (recency → severity → doc_id)
- [x] 4.3 Overlays (`Decisions`, `LawsAndViolations`, `GraphNeighbors`, `SimilarTurns`) gated by the request's `include` array; budget-pressure flips `debug.truncated = true`

## 5. Cache + audit
- [x] 5.1 In-memory cache keyed on `sha256(intent || scope || query || schema_version)` with the spec-11 10-min TTL; trait-shaped so a Synap-KV backend slots in identically
- [x] 5.2 `QueryService::invalidate_repo` drops every entry whose `scope.repo` matches; the wiring against the `cortex.cache.invalidate` consumer lands with the storage-layer dashboard work
- [x] 5.3 Per-request audit envelope on `cortex.events.query_audit` carrying `caller`, `query_id`, `intent`, `scope`, per-field counts, `latency_ms`, `cache`

## 6. ACL + rate limiter + final redaction
- [x] 6.1 Per-caller `AclStore` (allow-list + wildcard); 403 `scope_forbidden` on out-of-scope; `Unknown` callers fall through to `Allow`
- [x] 6.2 Token-bucket `RateLimiter` (30 rps sustained / 60 rps burst) with `Retry-After` header; 429 `rate_limited` body shape
- [x] 6.3 Final read-time redaction pass on every string-bearing response field via `cortex_core::redact::redact`; redaction count surfaces under `debug.errors.redacted`

## 7. Observability
- [x] 7.1 Per-lane timings on `debug.lanes.{vector_ms, keyword_ms, graph_ms}`; per-request audit envelope carries the same numbers
- [x] 7.2 `debug.errors.{vector,keyword,graph}` populated on partial failures + `debug.truncated` on budget-pressure soft-cancels

## 8. Tail (mandatory)
- [x] 8.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` flipped to 🟢 Implemented; `docs/specs/00-index.md` row updated to 🟢
- [x] 8.2 Write tests covering the new behavior — `tests/http.rs` (13) covers `pre_change_context` returning a snippet, cache hit labelled `hit`, empty-query 400, ACL deny 403, rate-limited 429 with `Retry-After`, lane failure not blocking other lanes, budget-pressure flipping `debug.truncated`, cache invalidation per repo, audit emitting one envelope per request, `law_check` returning only violations, redaction stripping a synthetic AWS-key from a snippet, MCP tool descriptor advertising the schema, and `mcp_invoke` routing through the same service. Lib unit tests (35) cover types defaults, intent JSON round-trips, lane traits, strategy plans per intent, RRF fusion + tie-breaks (recency + severity + doc_id), TTL expiry + repo invalidation, ACL allow / deny / wildcard, rate-limiter admit / refill / per-caller bucket isolation, redaction count surface, audit envelope shape, MCP descriptor, and the `QueryService` end-to-end pipeline (empty query + ACL deny + cache hit + rate limit + audit publish)
- [x] 8.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `cargo test -p cortex-api` all green (48 tests)
