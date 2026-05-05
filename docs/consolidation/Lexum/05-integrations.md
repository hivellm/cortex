# Lexum Integrations

## Cortex Search Choice: Meilisearch vs Lexum

### Current State
Cortex currently uses **Meilisearch** for full-text indexing, NOT Lexum.

### Why Meilisearch Over Lexum (at time of decision)

1. **Maturity**: Meilisearch 1.x is production-grade; Lexum is 0.1.0-alpha
2. **Stability**: Meilisearch has proven track record; Lexum still completing Phase 1
3. **Deployment**: Meilisearch Docker image widely available; Lexum Dockerfile recently added
4. **API Stability**: Meilisearch API stable; Lexum API still subject to change
5. **Feature Completeness**: Meilisearch has all core features; Lexum still adding distributed features
6. **Team Maturity**: Meilisearch team battle-tested; Lexum team still developing

### Switching to Lexum: Requirements

For Cortex to migrate from Meilisearch to Lexum, these items must be addressed:

#### 1. **Stability & Maturity**
- [ ] Complete Phase 2 (distributed clustering, aliases, reindexing)
- [ ] 6+ months production deployment proven
- [ ] API freeze (no breaking changes to 39 endpoints)
- [ ] Comprehensive upgrade path documentation

#### 2. **Feature Parity**
- [ ] Advanced aggregations (implemented in Phase 3)
- [ ] Distributed sharding/replication (Phase 2, 10% done)
- [ ] Index aliases and reindexing operations
- [ ] Full protocol support (MCP, UMICP working)
- [ ] Equivalent search quality to Meilisearch

#### 3. **Performance Validation**
- [ ] Benchmark against Cortex's ingestion patterns (17 repos)
- [ ] Measure query latency at scale (10K+ docs/sec)
- [ ] Memory and disk footprint comparison
- [ ] Concurrent query throughput testing

#### 4. **Operational Readiness**
- [ ] Production-grade monitoring (telemetry complete)
- [ ] Backup/restore procedures documented
- [ ] Capacity planning guide
- [ ] Troubleshooting documentation
- [ ] Runbook for common failures

#### 5. **Deployment & Integration**
- [ ] Docker Compose multi-node template
- [ ] Kubernetes Helm chart (in progress, 75% done)
- [ ] Docker image optimization (currently 2-3x Meilisearch size)
- [ ] Health check reliability
- [ ] Configuration management

#### 6. **Migration Path from Meilisearch**
- [ ] Data transformation tooling (Meili → Lexum schema mapping)
- [ ] Reindex operation with zero downtime
- [ ] Rollback procedure for failed migrations
- [ ] Dual-write period for validation
- [ ] Testing in Cortex's staging environment

## Why Lexum Over Alternatives

### vs Elasticsearch
- ✅ Lower memory footprint (Rust vs Java)
- ✅ Faster startup, no GC pauses
- ❌ Still immature (alpha vs production)
- ❌ Fewer plugins and integrations

### vs Meilisearch
- ✅ Distributed from ground up (Meilisearch less mature here)
- ✅ Custom query language (LQL) vs JSON DSL
- ✅ More control over indexing and scoring
- ❌ Less polished developer experience
- ❌ Smaller community

### vs Tantivy (direct)
- ✅ Complete server stack (not just library)
- ✅ REST API, clustering, replication
- ✅ Multiple query languages (SQL-like)
- ❌ Lower-level than Tantivy, less community

## Integration Points with Cortex

### Required Integrations
1. **Document Ingestion**: 17 repos → Lexum indices
2. **Query Interface**: Cortex frontend → Lexum search API
3. **Metadata**: Nexus graph node ids ↔ Lexum doc storage
4. **Vectorization**: Cortex-embedder output → Lexum vectors (phase)
5. **Analytics**: Cortex queries → Lexum aggregations

### Proposed Architecture
```
Cortex Walker/Classifier
    ↓
Document Preparation (schema mapping)
    ↓
Lexum REST API (/bulk or /index)
    ↓
Lexum Storage (Tantivy indices)
    ↓
Cortex Frontend
    ↓
Lexum Search API (LQL or REST)
    ↓
Results → Cortex UI
```

### Schema Mapping (Cortex ↔ Lexum)

**Cortex Document Properties → Lexum Fields**
```
{
  "source_repo": "cortex-core",       → keyword
  "file_path": "src/lib.rs",          → text
  "content": "fn main() { ... }",      → text (full-text)
  "language": "rust",                 → keyword
  "extracted_at": 2024-10-25T...,    → date
  "nexus_node_id": "external-id-123", → keyword
  "metadata": {...}                   → nested (optional)
}
```

### LQL Query Examples for Cortex

```sql
-- Find all Rust code mentioning "async"
FROM code_index
| WHERE language = "rust"
| MATCH "async" IN content
| LIMIT 100

-- Documents from cortex-core modified in Oct 2024
FROM code_index
| WHERE source_repo = "cortex-core"
|   AND extracted_at >= "2024-10-01"
| SORT extracted_at DESC

-- Aggregation: count by repo and language
FROM code_index
| GROUP BY source_repo, language
| COUNT
```

## MCP Integration (Planned)

### Lexum MCP Operations
```json
{
  "method": "lexum.search",
  "params": {
    "index": "cortex_knowledge",
    "query": "What is the async pattern here?",
    "k": 10,
    "filter": {"language": "rust"}
  }
}
```

## Vector Search Integration (Phase Future)

When Lexum adds vector search support:
- Accept embeddings from cortex-embedder
- Store vectors alongside text
- Hybrid search (BM25 + semantic)
- Relevance fusion

## Operational Considerations

### Deployment Size
- Single node: 1GB RAM, 2GB disk (basic setup)
- Multi-node cluster: 4GB+ per node recommended
- Data volume scales with indexed document count

### Monitoring Integration
- Prometheus metrics at `/_metrics` endpoint
- Can integrate with Cortex telemetry stack
- OpenTelemetry support planned (Phase 3)

### Backup Integration
- Snapshot API for point-in-time backups
- S3 support for remote storage
- Can be part of Cortex backup strategy
