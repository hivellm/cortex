# Nexus: Integrations

## HiveLLM Ecosystem Position

Nexus serves as the **graph persistence and query engine** for HiveLLM projects that need structured relationships + semantic embeddings. It complements other services:

| Service | Role | Integration Point |
|---------|------|-------------------|
| **Vectorizer** | Embedding generation (text → vector) | Nexus ingests embeddings, stores in HNSW KNN indexes |
| **Synap** | Key-value store (state snapshots) | Orthogonal; Nexus reads/stores application state in graphs |
| **Lexum** | Full-text search (document index) | Nexus has native FTS (Tantivy); can delegate specialized searches |
| **Expert** | LLM multi-turn reasoning | Nexus provides context graphs for Expert queries |
| **Rulebook** | Task management & memory | Nexus can index task graphs, decision records, learnings |

## Cortex Integration (cortex-graph module)

**Status**: Active (phase11l external-IDs migration gate met; phase 9 + 10 complete in Nexus 2.1.0)

### Data Flow

1. **Cortex ingests** from 17 HiveLLM repos (codebase analysis, metadata)
2. **cortex-graph** transforms structured data → Nexus external-ID format
3. **Nexus stores** with deterministic external IDs for idempotency
4. **Cortex queries** via `execute_cypher()` SDK calls (Rust SDK pinned to 2.1.0+)

### Key Artifacts

- **cortex-graph crate** — Graph construction & schema mapping
- **Nexus SDK binding** — `nexus-graph-sdk` dependency
- **External ID strategy** — Files/functions use SHA256(path+content) for stable identity
- **Conflict policy** — MATCH for incremental re-indexing (deterministic re-runs)

### Ingestion Pipeline

```
Source repos → cortex-classifier → cortex-embedder
                                       ↓
                                Vectorizer embeddings
                                       ↓
                              cortex-graph transforms
                                       ↓
                          Nexus (create_node_with_external_id)
                                       ↓
                            Graph stored + indexed
```

### Query Examples (from Cortex)

```cypher
-- Find all code files in a repository
MATCH (file:File {_id: 'sha256:...'})
WHERE file.repo = 'cortex'
RETURN file.path, file.size

-- Hybrid: find similar files + their imports
CALL vector.knn('File', $query_embedding, 10)
YIELD node AS file, score
MATCH (file)-[:IMPORTS]->(dep:File)
RETURN file._id, file.path, dep.path, score
ORDER BY score DESC

-- Knowledge graph: trace dependencies
MATCH (repo:Repository {name: 'cortex'})
MATCH (repo)-[:CONTAINS]->(file:File)
MATCH (file)-[:IMPORTS*1..3]->(downstream:File)
RETURN repo.name, file.path, downstream.path
```

## SDK Compatibility (phase 10 validation)

All six first-party SDKs ship `create_node_with_external_id` + `get_node_by_external_id`:

- **Rust** (`nexus-graph-sdk`): Used by cortex-graph directly
- **Python** — available for cortex-worker (future)
- **TypeScript** — available for cortex-api (future)
- **Go** — available for Go-based workers
- **C#** — available for .NET integrations
- **PHP** — available for PHP-based consumers

**Validation status**: 87/87 live SDK test cases on `hivehub/nexus:2.1.0` image (phase 10 complete)

## Operational Integration

### Docker Deployment
```bash
# Latest stable (2.1.0+)
docker run -p 15474:15474 -p 15475:15475 -v nexus-data:/data \
  hivehub/nexus:2.1.0

# Development (latest)
docker run -e NEXUS_AUTH_ENABLED=false \
  hivehub/nexus:latest
```

### Environment Variables (Cortex relevance)
- `NEXUS_ADDR` — HTTP bind (default: 127.0.0.1:15474)
- `NEXUS_RPC_ADDR` — RPC bind (default: 127.0.0.1:15475)
- `NEXUS_DATA_DIR` — storage path (default: ./data)
- `NEXUS_AUTH_ENABLED` — require auth (true for 0.0.0.0)
- `NEXUS_SIMD_DISABLE` — emergency scalar fallback

### Helm Chart (Kubernetes)
- **Chart**: `deploy/helm/nexus/`
- **Topologies**: single-node, master-replica, v2-cluster
- **App version**: synced to Nexus version (2.1.0 → `appVersion: 2.1.0`)

## Configuration & Tuning

### For Cortex Use Case (High Read Volume + Graph Traversal)
1. **KNN indexes**: Enable HNSW on Document/File/Code labels (phase11l task)
2. **Label bitmap cache**: Large pool for fast label scans
3. **Page cache**: Tune for hot-path node/relationship lookups
4. **Connection pool**: RPC multiplexing supports pipelined queries
5. **Rate limiting**: Configure per-key quotas (default: 1k/min, 10k/hour)

### Database Isolation
- **Multi-database support** — cortex could use separate database (`USE cortex`)
- **Catalog per-DB** — external IDs, indexes, constraints scoped per database
- **Replication** — master-replica for read scaling (replicas handle cortex queries)

## Monitoring & Observability

### Prometheus Metrics (exposed at `/prometheus`)
- `nexus_query_count` — labeled by statement type
- `nexus_cache_hit_ratio` — L1/L2/L3 layers
- `nexus_index_lookup_latency_ms` — histogram
- `nexus_rpc_connections` — active connections
- `nexus_audit_log_failures_total` — fail-open count

### Health Check
- **GET** `/health` — liveness probe (Kubernetes readiness)
- **Output**: JSON with status, version, connection pool state

### Replication Lag
- **GET** `/replication/lag` — real-time master-replica offset (if running replicated)

## Future Integration Paths

1. **Streaming Cypher** — RPC push frames reserved (PUSH_ID = u32::MAX); no SDK consumer yet
2. **MCP server** — Nexus itself as an MCP tool for LLMs (cortex-api bridge)
3. **RESP3 in non-Rust SDKs** — RESP3 parser/writer for Python, TypeScript (queued)
4. **Online re-sharding** — V2.1+ feature for transparent scale-out (cortex cluster mode)

## Known Limitations (Cortex context)

- **Not a document store** — use Vectorizer or Elasticsearch for pure document search
- **Write-heavy workloads** — single-writer + MVCC favors reads; writes go through one thread
- **Embedded mode not available** — Nexus is server-only; requires TCP connection
- **KNN recall tuning** — HNSW parameters (M, ef) must be tuned per label + dimensionality
