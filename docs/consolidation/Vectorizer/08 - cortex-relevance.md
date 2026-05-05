# Vectorizer — Cortex Integration & Ingestion Priority

**Last Updated:** 2026-05-04

## What Cortex Should Ingest

### Tier 1: Critical (Ingestion Priority = HIGH)

These are the essential pieces of knowledge for Cortex to integrate with Vectorizer and troubleshoot issues.

**1. Auth Resolution & Credential Hierarchy**
- Source: `docs/operations/vectorizer-auth.md` (Cortex repo, already exists)
- Cortex dependency: `cortex-api` must boot with valid Vectorizer credentials
- Knowledge needed: Precedence order (API key > user+pass > alias > anonymous)
- Ingestion value: Bootstrap troubleshooting, JWT refresh strategy, warmup loop
- **File:** `03 - public-surface.md` (auth section) + `07 - operational.md` (config section)

**2. Vector Lane API & Search Methods**
- Source: `crates/vectorizer-server/src/api/` (Vectorizer repo)
- Cortex dependency: `VectorizerLane` calls `search()`, `search_hybrid()`, `multi_collection_search()`
- Knowledge needed: Method signatures, error codes, rate limiting
- Ingestion value: Understand vector lane failure modes, implement retry logic
- **File:** `03 - public-surface.md` (REST API section)

**3. Collection Naming Conventions**
- Source: `docs/specs/WORKSPACE.md` (Vectorizer repo)
- Cortex dependency: Know which collections to query for decisions, context, metadata
- Knowledge needed: Naming scheme (e.g., `cortex-decisions`, `cortex-metadata`)
- Ingestion value: Avoid collisions, organize Cortex-specific collections
- **File:** `04 - data-and-storage.md` (collection metadata section)

**4. Hybrid Search (RRF) & Ranking**
- Source: `docs/specs/INTELLIGENT_SEARCH.md` (Vectorizer repo)
- Cortex dependency: Decision context retrieval uses `search_hybrid()`
- Knowledge needed: BM25 sparse + HNSW dense + RRF fusion
- Ingestion value: Tune sparse/dense blend for decision quality
- **File:** `02 - architecture.md` (search pipeline section)

**5. Graph Relationships & Discovery**
- Source: `docs/specs/GRAPH_RELATIONSHIPS.md` (Vectorizer repo)
- Cortex dependency: Multi-hop decision navigation, related issues discovery
- Knowledge needed: Enable graph, edge types, discovery pipeline
- Ingestion value: Traverse decision relationships, enrich context
- **File:** `02 - architecture.md` (graph section) + `03 - public-surface.md` (graph endpoints)

### Tier 2: Important (Ingestion Priority = MEDIUM)

These help optimize performance and understand trade-offs.

**6. HNSW Tuning & Performance**
- Source: `docs/specs/PERFORMANCE.md` (Vectorizer repo)
- Cortex dependency: Decision search latency matters (every query blocks decision engine)
- Knowledge needed: ef_search parameter, cache sizing, latency benchmarks
- Ingestion value: Optimize search speed for critical path
- **File:** `02 - architecture.md` (HNSW indexing section) + `07 - operational.md` (monitoring)

**7. Quantization & Memory Pressure**
- Source: `docs/specs/PQ_IMPLEMENTATION.md` (Vectorizer repo)
- Cortex dependency: Large decision index may trigger OOM on resource-constrained environments
- Knowledge needed: PQ compression (64x), SQ (4x), memory-mapping
- Ingestion value: Reduce memory footprint, decide compression strategy
- **File:** `02 - architecture.md` (quantization section) + `04 - data-and-storage.md` (storage optimization)

**8. Replication & HA**
- Source: `docs/specs/REPLICATION.md` (Vectorizer repo)
- Cortex dependency: Production Cortex may need HA (automatic failover, read replicas)
- Knowledge needed: Raft vs Master-Replica, failover process, WAL recovery
- Ingestion value: Plan HA strategy, understand recovery time
- **File:** `02 - architecture.md` (persistence section) + `07 - operational.md` (scaling & HA)

**9. Docker Deployment & Profiles**
- Source: `docker-compose.yml` (Vectorizer repo)
- Cortex dependency: Cortex boots Vectorizer in Docker; must understand profiles (default/dev/ha/hub)
- Knowledge needed: Port bindings, volume mounts, environment variables
- Ingestion value: Configure Cortex stack correctly, avoid profile conflicts
- **File:** `07 - operational.md` (Docker deployment section)

### Tier 3: Optional (Ingestion Priority = LOW)

These are nice-to-have for specialized use cases.

**10. MCP Tools & AI IDE Integration**
- Source: `docs/specs/MCP.md` (Vectorizer repo)
- Cortex dependency: None (Vectorizer feature, not used by Cortex core)
- Knowledge needed: 31 registered tools, StreamableHTTP
- Ingestion value: Enable developer workflows (code search, document discovery)
- **File:** `03 - public-surface.md` (MCP section) + `01 - overview.md` (mentions MCP)

**11. SDK Routing & RPC vs REST**
- Source: `docs/specs/SDK_ROUTING_ARCHITECTURE.md` (Vectorizer repo)
- Cortex dependency: Cortex uses `vectorizer-sdk` (RPC or REST transparently)
- Knowledge needed: URL parsing (vectorizer:// vs http://), protocol negotiation
- Ingestion value: Understand SDK behavior, debug protocol issues
- **File:** `03 - public-surface.md` (VectorizerRPC section) + `02 - architecture.md` (transport section)

**12. Qdrant Migration Tools**
- Source: `docs/specs/QDRANT_MIGRATION.md` (Vectorizer repo)
- Cortex dependency: None (data import tool, not runtime dependency)
- Knowledge needed: Config migration, data export/import, validation
- Ingestion value: Evaluate Qdrant → Vectorizer migration cost
- **File:** `04 - data-and-storage.md` (migration section)

## Cortex-Specific Ingestion Recommendations

### Must Read (Before Integration)

1. **`01 - overview.md`** (5 min) — Understand what Vectorizer is and where it fits in HiveLLM
2. **`05 - integrations.md`** (10 min) — See how Cortex, CompressionPrompt, Nexus, Synap interact with Vectorizer
3. **`03 - public-surface.md` (REST API + MCP sections)** (10 min) — Learn the endpoint signatures Cortex will call
4. **`07 - operational.md` (Docker + auth sections)** (10 min) — Understand how to boot and configure Vectorizer for Cortex

### Should Read (During Development)

5. **`02 - architecture.md` (search pipeline + graph sections)** (15 min) — Understand how decision retrieval works
6. **`06 - decisions-and-rationale.md` (hybrid search + graph sections)** (10 min) — Understand design trade-offs
7. **`04 - data-and-storage.md` (collection schema section)** (5 min) — Design decision payload structure

### Nice-to-Have (For Optimization)

8. **`07 - operational.md` (monitoring + troubleshooting)** (10 min) — Debug production issues
9. **`02 - architecture.md` (HNSW + quantization sections)** (15 min) — Optimize search latency and memory
10. **`08 - cortex-relevance.md`** (this file) (5 min) — Know what knowledge matters for Cortex

## Integration Checklist

- [ ] Boot Vectorizer container with Cortex-specific credentials (`CORTEX_VECTORIZER_USER` + `_PASSWORD`)
- [ ] Create `cortex-decisions` collection (embedding type: hybrid or dense)
- [ ] Enable graph for decision relationships (if multi-hop navigation needed)
- [ ] Index first 100 decisions, verify search latency < 3ms
- [ ] Configure Cortex vector lane with Vectorizer URL + auth
- [ ] Test decision context retrieval (`search_hybrid()` call)
- [ ] Set up monitoring: query latency, collection size, cache hit ratio
- [ ] Plan HA: decide on Raft cluster vs standalone (based on availability requirements)
- [ ] Document in Cortex runbooks: Vectorizer bootstrap, troubleshooting, scaling

## Known Cortex ↔ Vectorizer Touchpoints

| Cortex Module | Vectorizer Call | Purpose | Latency SLA |
|---------------|-----------------|---------|-------------|
| `cortex-api` vector lane | `search_hybrid()` | Decision context retrieval | < 3ms p95 |
| Decision engine | `multi_collection_search()` | Find related decisions | < 10ms p95 |
| Graph enrichment | `graph_get_neighbors()` | Related issue traversal | < 5ms p95 |
| Audit log | `get_collection_stats()` | Stats export | < 100ms p95 |
| Auth | `POST /auth/refresh` | JWT renewal on 401 | < 100ms p95 |

## Potential Cortex Enhancements

1. **Cached search expansion** — cache frequent decision query expansions locally
2. **Ranked suggestions** — use Vectorizer graph to rank decision suggestions by relevance
3. **Incremental indexing** — stream new decisions to Vectorizer as they're created (not batch)
4. **Multi-model scoring** — blend multiple embedding models for better decision ranking
5. **Temporal decay** — weight older decisions lower, recent ones higher (similarity + time)

## Monitoring & Alerting (Cortex Operations)

**Health checks:**
- `GET http://vectorizer:15002/health` (no auth, < 100ms expected)
- `POST /collections/cortex-decisions/stats` (verify vector count > 0)

**Alerts to set:**
- Vector lane 401s (auth failure) → check credentials
- Search latency > 10ms p95 (performance degradation) → increase cache, tune ef_search
- Collection size > 1M vectors (consider sharding) → enable distributed clustering
- Raft leader election failures (cluster unhealthy) → manual failover
- Low cache hit ratio (< 50%) → increase `memory.max_cache_memory_bytes`

## See Also

- `docs/operations/vectorizer-auth.md` (Cortex repo) — detailed auth resolution
- `crates/cortex-api/src/vector_lane.rs` (Cortex repo) — Cortex-side integration code
- `docs/specs/VECTORIZER_RPC.md` (Vectorizer repo) — low-level protocol for debugging
- `.rulebook/PLANS.md` (Cortex repo) — session context on Cortex phases
