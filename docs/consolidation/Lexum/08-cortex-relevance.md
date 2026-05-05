# Lexum Relevance to Cortex

## Why Cortex Should Track Lexum

1. **Search Engine Alternative**: Currently uses Meilisearch; Lexum offers distributed alternative
2. **HiveLLM Project**: Part of same ecosystem (Vectorizer, Nexus, Synap, Expert, Rulebook)
3. **Integration Path**: Can replace Meili if Phase 2 completes and performance proven
4. **Shared Vision**: Both target modern, cloud-native LLM-aware applications

## Cortex's Current Stack

- **Search**: Meilisearch (for full-text indexing of 17 repos)
- **Graph**: Nexus (external node IDs)
- **Embeddings**: Vectorizer (embedding generation)
- **Knowledge**: Synap (knowledge graph integration)
- **Pipeline**: Classifier → Meili index → Cortex UI

## Potential Lexum Integration Points

### 1. Document Indexing (Highest Priority)
**Current**: Cortex walker → Meilisearch
**Proposed**: Cortex walker → Lexum REST API

**Requirements**:
- [ ] Lexum API stability guarantee
- [ ] Schema mapping (Cortex doc format → Lexum schema)
- [ ] Bulk ingestion optimization
- [ ] Index template support

**Data Flow**:
```
Cortex Document
├── source_repo (keyword)
├── file_path (text)
├── content (text full-text)
├── language (keyword)
├── extracted_at (date)
└── nexus_node_id (keyword for linking)
    ↓
Lexum Index
    ↓
Cortex Query Interface
```

### 2. Search Queries (High Priority)
**Current**: Meilisearch query API
**Proposed**: Lexum LQL or REST search API

**Requirements**:
- [ ] LQL parser handles Cortex query patterns
- [ ] Full-text quality comparable to Meili
- [ ] Aggregations for analytics (repos, languages, etc.)
- [ ] Pagination and sorting

**Example Lexum Queries for Cortex**:
```sql
-- Search across all repos
FROM code_index | MATCH "async" IN content | LIMIT 50

-- Filter by repo and language
FROM code_index 
| WHERE source_repo = "cortex-core" AND language = "rust"
| MATCH "query" IN content

-- Analytics: document count by repo
FROM code_index | GROUP BY source_repo | COUNT
```

### 3. Vector Search Integration (Medium Priority - Phase Future)
**Status**: Not yet implemented in Lexum
**Cortex Need**: Semantic search on extracted content

**Proposed Architecture**:
```
cortex-embedder (Vectorizer)
    ↓ (embeddings)
Lexum (hybrid index: BM25 + vectors)
    ↓
Cortex UI (combined results)
```

### 4. Nexus Graph Integration (Medium Priority)
**Current**: Cortex references Nexus external node IDs
**Proposed**: Lexum documents link back to Nexus graph

**Mechanism**:
- Store `nexus_node_id` as keyword field in Lexum
- Query Nexus for relationships: document → node → related_nodes
- Enrich Cortex results with graph context

**Example**:
```
Lexum Search Result
  doc_id: 123
  nexus_node_id: "node-abc"
    ↓
Nexus /nodes/by-external-id?id=node-abc
    ↓
Related nodes, properties, edges
    ↓
Cortex enriched UI
```

## Ingestion Priorities (If Switching)

### Phase 1: Prepare (2 weeks)
1. Define Cortex document schema for Lexum
2. Implement schema builder in Cortex codebase
3. Create bulk ingestion client
4. Performance test against representative data

### Phase 2: Migrate (2-3 weeks)
1. Dual-write period (both Meili and Lexum)
2. Validate search results equivalence
3. Establish rollback procedure
4. Staging environment full deployment

### Phase 3: Cutover (1 week)
1. Switch Cortex query layer to Lexum
2. Verify production metrics
3. Monitor query latency, accuracy
4. Decommission Meilisearch

## Required from Lexum Before Cortex Switch

### Must-Have (Blocker)
- [x] REST API (39 endpoints working)
- [x] Full-text search (BM25 scoring)
- [x] Bulk document operations
- [x] Query filtering and aggregations
- [ ] **Phase 2 completion** (distributed, stable)
- [ ] **6+ months production proof** (other users)
- [ ] **API stability guarantee** (no breaking changes)

### Should-Have (Risk Mitigator)
- [ ] Advanced aggregations (completed)
- [ ] Index reindexing without downtime
- [ ] Horizontal scaling (Phase 2)
- [ ] Improved Docker image size/startup
- [ ] Kubernetes Helm chart (in progress, 75%)
- [ ] Production telemetry (OpenTelemetry Phase 3)

### Nice-to-Have (Enhancements)
- [ ] Vector search support
- [ ] MCP integration for Cortex brain
- [ ] Multi-tenancy (for multi-workspace Cortex)
- [ ] Cached query results (performance)

## Risk Assessment

### Low Risk
- REST API is stable (39 endpoints tested)
- Core search engine solid (Tantivy proven)
- Can run alongside Meilisearch (dual-write period)
- Rollback simple (revert to Meili) if issues found

### Medium Risk
- Alpha status (not production yet)
- Distributed features untested (Phase 2)
- Smaller community than Elasticsearch/Meili
- Docker image optimization needed

### High Risk
- Cortex depends on search stability
- Performance regression would impact UX
- Migration testing must be thorough
- Team bandwidth (if Lexum issues arise)

## Timeline Estimate

| Milestone | When | Effort | Blocker |
|-----------|------|--------|---------|
| Lexum Phase 1 complete | ✅ Done | — | — |
| Lexum Phase 2 (distributed) | Q4 2025 | 6-8 weeks | Yes |
| 6-month production proof | Q1 2026 | — | Yes |
| Cortex dual-write test | Q2 2026 | 1 week | No |
| Cortex staging cutover | Q2 2026 | 1 week | No |
| Cortex production switch | Q3 2026 | 1 day | No |

## Recommendation

**For Cortex phase11l→**: 
- Continue using Meilisearch (proven, stable)
- Monitor Lexum progress (Phase 2 completion, production users)
- Plan dual-write proof-of-concept for Q2 2026
- Commit to Lexum migration Q3 2026 if Phase 2 meets quality bar

**Rationale**:
1. Meilisearch already works (no disruption now)
2. Lexum Phase 2 timeline aligns with Cortex maturity
3. De-risked evaluation period (6+ months)
4. Cortex can drive Lexum Phase 2 requirements if committed

## Cortex-Lexum Collaboration Opportunities

1. **Cortex as Phase 2 Driver**: Use Cortex needs to prioritize distributed, aggregations, performance
2. **Shared Testing**: Cortex ingestion as load test for Lexum
3. **Documentation**: Cortex use case in Lexum docs/tutorials
4. **Roadmap Alignment**: Cortex vector search needs → Lexum Phase Future
5. **Team Learning**: Cross-project knowledge sharing (Rust, distributed systems)
