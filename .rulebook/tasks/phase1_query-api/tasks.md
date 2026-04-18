## 1. Crate scaffold
- [ ] 1.1 `cortex-api` crate (Axum + MCP binding)
- [ ] 1.2 `POST /v1/query` with request/response types per spec 11
- [ ] 1.3 MCP tool `cortex.query` exposing the same schema

## 2. Strategies
- [ ] 2.1 Intent → execution-plan mapper for `pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `free_search`
- [ ] 2.2 Scope canonicalization + validation
- [ ] 2.3 Per-intent lane + overlay set defined in `orchestrator/strategies.rs`

## 3. Lane clients
- [ ] 3.1 Vectorizer KNN client (top-k per collection; multi-collection fan-in)
- [ ] 3.2 Meilisearch client (filterable search; sort by ts; typo tolerance)
- [ ] 3.3 Nexus Cypher client (parametrized read-only queries)

## 4. Orchestrator
- [ ] 4.1 Parallel fan-out with `tokio::join_all`; sub-budgets 40/40/20
- [ ] 4.2 RRF fusion (`score = Σ 1/(60+rank)`); tie-break on recency then severity
- [ ] 4.3 Overlays (decisions, laws, graph_neighbors, similar_turns) gated by `include[]`

## 5. Cache + audit
- [ ] 5.1 Cache key `hash(intent || scope || query_embedding || schema_version)`; Synap KV TTL 10 min
- [ ] 5.2 Invalidate on `cortex.cache.invalidate` events scoped by repo
- [ ] 5.3 Per-request audit event to `cortex.events.query_audit` with caller, intent, scope, counts, latency

## 6. ACL + rate limiter + final redaction
- [ ] 6.1 Per-caller ACL store (`caller → allowed_repos`); 403 on out-of-scope
- [ ] 6.2 Token-bucket rate limiter (30 rps sustained / 60 rps burst) with `Retry-After`
- [ ] 6.3 Final redaction pass on response bodies (belt-and-suspenders)

## 7. Observability
- [ ] 7.1 Counters + histograms per spec 11 §Observability
- [ ] 7.2 `debug.lanes.*` + `debug.errors.*` populated on partial failures

## 8. Tail (mandatory)
- [ ] 8.1 Update `docs/specs/11-query-api.md` status flag to 🟢 + index row
- [ ] 8.2 Integration tests: `pre_change_context` returns ≥1 snippet within 500 ms; cache hit <20 ms; 200 ms Nexus stall does not blow the budget; RRF golden-set precision ≥0.7; budget_ms=100 truncates with `debug.truncated=true`; overlays attach in-scope decisions; similar-turns KNN ordering; `law_check` returns only violations; MCP binding; ACL deny; rate-limit 429; redaction of embedded synthetic secret; lane-off partial response; cache invalidation on critical ingestion event
- [ ] 8.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
