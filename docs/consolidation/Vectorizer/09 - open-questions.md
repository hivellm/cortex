# Vectorizer — Open Questions & Known Gaps

**Last Updated:** 2026-05-04

## Architectural Unknowns

### 1. GPU Support & Roadmap Clarity

**Question:** When will Vectorizer support CUDA (beyond Metal on macOS)?

**Current State:**
- Metal GPU support (v3.3.0) via `hive-gpu` crate (macOS only, Apple Silicon)
- CPU inference only (Candle / FastEmbed)
- No CUDA roadmap published

**Impact on Cortex:**
- Cannot accelerate embedding inference on Nvidia GPUs
- CPU-bound for large indexing workloads
- Metal GPU only benefits macOS deployments

**Action Items:**
- [ ] Clarify CUDA support timeline with Vectorizer team
- [ ] Evaluate performance impact (CPU vs GPU) for Cortex scale
- [ ] Consider hybrid: use Vectorizer for indexing, external embedding service for inference (if CUDA blocks)

### 2. Distributed Graph Relationships

**Question:** When will graph relationships work on sharded collections?

**Current State:**
- Graph is CPU-only (GPU/sharded collections explicitly rejected)
- Graph is per-collection in-memory
- No distributed graph traversal across shards

**Impact on Cortex:**
- Cannot navigate decision relationships in multi-shard setup
- HA Cortex may need workaround (centralized decision graph index outside Vectorizer)
- Sharding becomes less valuable for decision networks

**Action Items:**
- [ ] Decide: local graph (per shard) vs distributed graph (centralized)?
- [ ] If sharding required, build Cortex-side graph cache or use Nexus for relationships
- [ ] Monitor Vectorizer roadmap for distributed graph support

### 3. Embedding Model Versioning

**Question:** How does Vectorizer handle embedding model version upgrades?

**Current State:**
- Models bundled at build time (Candle / FastEmbed)
- No online model updates (binary update required)
- No version tracking in `.vecdb` metadata

**Impact on Cortex:**
- Cannot A/B test embedding models without reindexing
- Model drift across Vectorizer versions
- Decision vectors may become incompatible after upgrade

**Action Items:**
- [ ] Add model version to collection metadata
- [ ] Implement embedding compatibility layer (upcast old vectors to new model space)
- [ ] Document: reindexing procedure after major upgrades

### 4. Raft Split-Brain Scenarios

**Question:** How does Vectorizer recover from Raft split-brain (multiple leaders)?

**Current State:**
- Raft consensus implemented (openraft 0.10.0-alpha.17)
- No published documentation on split-brain behavior
- Unclear if manual intervention required

**Impact on Cortex:**
- Uncertain RTO (Recovery Time Objective) during network partitions
- Risk of data divergence across partitions
- May require manual failover

**Action Items:**
- [ ] Request Vectorizer split-brain runbook
- [ ] Test failover scenario (stop leader, verify replica promotion)
- [ ] Decide: manual failover vs automated detection + leader demotion

### 5. Cross-Collection Transactions

**Question:** Does Vectorizer support atomic inserts across multiple collections?

**Current State:**
- Transactions are per-collection (WAL-driven)
- No cross-collection atomicity (A-B-C insertion could fail at B)
- Multi-collection search is read-only

**Impact on Cortex:**
- Decision metadata may be split across collections (decisions + context + graph edges)
- Partial indexing on failure (recovery needed)
- Eventual consistency gap during failures

**Action Items:**
- [ ] Design Cortex payload structure (avoid cross-collection dependencies)
- [ ] Implement idempotent insertion (safe to retry failed batches)
- [ ] Monitor collection consistency (periodic audit)

## Integration Unknowns (Cortex-Specific)

### 6. Vector Lane Latency Under High Load

**Question:** How does Vectorizer search latency degrade at Cortex peak throughput?

**Current State:**
- Benchmarks show < 3ms latency at ~4,400 QPS
- Cortex decision engine may drive 100+ decisions/sec (= hundreds of vector searches)
- Unclear how HNSW performance scales under sustained load

**Impact on Cortex:**
- Decision engine may stall on slow vector searches (cascade failure)
- Need circuit breaker or timeout handling

**Action Items:**
- [ ] Load test: 1,000 concurrent decision queries
- [ ] Measure p99 latency (target: < 10ms)
- [ ] Configure timeout + fallback (MemoryVectorLane)
- [ ] Alert on latency > 5ms p95

### 7. Auth Credential Rotation Safety

**Question:** Can Cortex safely rotate Vectorizer API keys without downtime?

**Current State:**
- API key rotation supported (v3.3.0, 300s grace window)
- Cortex may cache old credentials in-process
- Unclear if grace window covers long-running transactions

**Impact on Cortex:**
- Potential to lose vector lane connectivity during key rotation
- In-flight searches may 401 mid-batch

**Action Items:**
- [ ] Test: rotate Cortex API key, verify no request loss
- [ ] Extend grace window to > max transaction time (default 300s)
- [ ] Implement: exponential backoff on 401 (retry with current key)

### 8. Payload Filtering Complexity

**Question:** What is the performance cliff for complex payload filters in searches?

**Current State:**
- Vectorizer supports boolean filter expressions (JSON path + operators)
- No published benchmarks on filter complexity
- May require full payload scan (no index on payload fields)

**Impact on Cortex:**
- Decision search with metadata filters (e.g., `timestamp > yesterday`) may be slow
- Nested filters (e.g., `(type=decision AND priority=high AND status!=resolved)`) unclear

**Action Items:**
- [ ] Benchmark: search with 1, 5, 10 filter conditions
- [ ] If slow (> 1ms per filter), denormalize decision payloads (use fields, not nested objects)
- [ ] Add indexed fields for common filters (timestamp, status, priority)

## Operational Unknowns

### 9. Backup & Restore Performance at Scale

**Question:** How long does backup/restore take for 10M+ vector datasets?

**Current State:**
- Backup API exists but no throughput benchmarks published
- Unclear if backup is incremental-friendly
- No guidance on backup frequency vs storage cost

**Impact on Cortex:**
- RPO (Recovery Point Objective) depends on backup frequency
- Large backups may consume bandwidth, delay retention cleanup

**Action Items:**
- [ ] Benchmark: backup 1M, 10M vectors (measure throughput, storage)
- [ ] Calculate: storage cost for daily backups (7, 30, 90 day retention)
- [ ] Decide: full daily vs incremental weekly + daily increment

### 10. Metrics & Alerting Gaps

**Question:** What observability is missing for production Vectorizer monitoring?

**Current State:**
- Prometheus metrics exported (latency, throughput, cache hit ratio)
- Missing: search result quality (recall vs baseline), cost per query
- Unclear: tail latency percentiles (p99.9)

**Impact on Cortex:**
- Cannot detect silent degradation (low recall, expensive queries)
- Difficult to correlate search quality to decision quality

**Action Items:**
- [ ] Add custom Cortex metrics (decision accuracy vs vector search quality)
- [ ] Export decision-level attribution (which vectors drove decision?)
- [ ] Monitor: search expansion ratio (query → N expanded queries) for drift

### 11. Multi-Tenant Isolation (HiveHub Cluster Mode)

**Question:** Is Vectorizer collection-scoped isolation sufficient for Cortex multi-tenancy?

**Current State:**
- HiveHub cluster mode isolates by collection (`tenant_isolation: "collection"`)
- Cortex may have shared collections (e.g., common decision patterns)
- Unclear if cross-tenant search queries are prevented

**Impact on Cortex:**
- Potential information leakage (tenant A sees tenant B decisions)
- Quota enforcement per collection (not cross-tenant)

**Action Items:**
- [ ] Review: HiveHub cluster mode security model
- [ ] Design: Cortex tenant isolation strategy (shared vs dedicated collections)
- [ ] Test: verify tenant A cannot read collection owned by tenant B

## Documentation Gaps

### 12. Missing Cortex Integration Examples

**Question:** Where are the reference examples for Cortex + Vectorizer integration?

**Current State:**
- `docs/users/` focuses on generic Vectorizer usage
- No Cortex-specific integration guide exists
- Examples are scattered across multiple docs

**Impact on Cortex:**
- New Cortex developers lack reference implementation
- Risk of suboptimal integration (wrong collection naming, search parameters)

**Action Items:**
- [ ] Create: `docs/examples/cortex-integration.md` (this doc set)
- [ ] Add: example Cortex collection schema (payloads, indexes)
- [ ] Add: example decision search query (hybrid parameters, filters)

### 13. Performance Tuning Guidance

**Question:** What are the tuning knobs for optimizing Vectorizer for Cortex workload?

**Current State:**
- HNSW parameters documented (M, ef_construction, ef_search)
- No Cortex-specific tuning guide
- Trade-offs between latency, memory, accuracy unclear

**Impact on Cortex:**
- Risk of suboptimal configuration
- May leave performance on the table (slow searches, OOM on scale)

**Action Items:**
- [ ] Create: tuning guide (HNSW parameters for decision workload)
- [ ] Recommend: default config (M=16, ef=200, ef_search=40) for Cortex scale
- [ ] Document: how to profile search latency by parameter

## Data Quality Unknowns

### 14. Vector Deduplication & Collision Handling

**Question:** How should Cortex handle duplicate decisions in the vector index?

**Current State:**
- Vectorizer allows duplicate vectors (same content, different IDs)
- Search returns all duplicates (no deduplication)
- Unclear if Cortex should filter duplicates or merge them

**Impact on Cortex:**
- Search results may include near-identical decisions
- UX confusion (user sees same decision twice)
- Inflated collection stats

**Action Items:**
- [ ] Design: deduplication strategy (hash-based, similarity threshold)
- [ ] Implement: Cortex-side deduplication filter in vector lane
- [ ] Monitor: duplicate rate (alert if > 5%)

### 15. Search Quality Degradation Over Time

**Question:** Does search quality degrade as collection grows?

**Current State:**
- HNSW is approximate (may miss neighbors as graph grows)
- No published recall benchmarks at various collection sizes
- ef_search tuning unclear for dynamic collections

**Impact on Cortex:**
- Decision discovery quality may degrade with time (more decisions indexed)
- May require periodic reindexing (no automated procedure)

**Action Items:**
- [ ] Benchmark: recall at 100K, 1M, 10M vectors
- [ ] Monitor: search quality (compute expected rank vs actual rank)
- [ ] Implement: periodic index rebuild (trigger if recall < 95%)

## Dependency Risks

### 16. openraft Stability (Raft Implementation)

**Question:** How stable is openraft 0.10.0-alpha.17?

**Current State:**
- Pinned to 0.10.0-alpha.17 (pre-release)
- 0.11+ has breaking changes (not yet migrated)
- Upstream support uncertain (alpha channel)

**Impact on Cortex:**
- Risk of abandoned Raft implementation
- May need emergency fork if upstream breaks

**Action Items:**
- [ ] Monitor: openraft releases and stability (move to stable when available)
- [ ] Plan: upgrade path to 0.11+ (when Vectorizer migrates)
- [ ] Evaluate: alternative Raft libraries (etcd, TiKV) if openraft unmaintained

### 17. Candle/FastEmbed Model Compatibility

**Question:** Will embedding models remain available in future Candle releases?

**Current State:**
- Candle 0.10.2 pins (may lag latest)
- FastEmbed models bundled (license compliance unknown)
- No published support policy

**Impact on Cortex:**
- Risk of model eviction (upgrade breaks embedding)
- Potential licensing issues (audit needed)

**Action Items:**
- [ ] License audit: verify all bundled models (commercial-compatible)
- [ ] Plan: upgrade Candle to next major version (evaluate impact)
- [ ] Evaluate: custom quantized models (reduce binary size, ownership)

## See Also

- `docs/future/FUTURE_ROADMAP.md` (Vectorizer repo) — published roadmap
- `.rulebook/PLANS.md` (Cortex repo) — Cortex development priorities
- `docs/specs/analysis/` (Vectorizer repo) — architectural analyses

---

**Last Review:** 2026-05-04  
**Next Review:** 2026-06-04 (quarterly check-in with Vectorizer team)
