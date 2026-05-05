# Lexum Open Questions & Gaps

## Architecture & Design

### Q1: Distributed Consensus Implementation
**Status**: Raft planned but not implemented (Phase 2, 10% done)

**Questions**:
- Which Raft library will be used? (raft-rs, tikv-raft, etc.)
- How does Raft mesh with existing RocksDB metadata storage?
- Failover time target for leader election?
- Split-brain recovery procedure?
- Network partition handling (quorum write availability)?

**Impact**: Critical for high-availability deployments

### Q2: Vector Search Support Timeline
**Status**: Not yet planned, mentioned in Phase Future

**Questions**:
- Will Lexum add native vector search or delegate to external service?
- Hybrid search scoring (BM25 + vector similarity)?
- Vector embedding ingestion API?
- HNSW or other ANN algorithm?

**Impact**: Blocks semantic search use cases (Cortex embeddings)

### Q3: Multi-Tenancy & Namespace Isolation
**Status**: Not addressed in current design

**Questions**:
- How to securely isolate indices per tenant?
- Tenant-level quota management?
- RBAC scoped to tenant boundaries?
- Shared cluster resource management?

**Impact**: Blocks multi-workspace Cortex deployments

## Performance & Scalability

### Q4: Horizontal Scaling Validation
**Status**: Designed but not proven at scale

**Questions**:
- Verified performance with 100M+ documents?
- Shard rebalancing overhead during scaling?
- Query latency degradation with N nodes?
- Network bandwidth requirements between nodes?
- Consistent hashing impact on write/read paths?

**Impact**: Unknown real-world scaling limits

### Q5: Indexing Throughput
**Status**: Target is 50K-100K docs/sec, not measured

**Questions**:
- Actual throughput in realistic workloads?
- Memory usage per ingested document?
- CPU overhead for analysis (tokenization, stemming)?
- Segment merge impact on write throughput?
- Bulk API batching recommendations?

**Impact**: Unknown suitability for Cortex 17-repo ingestion rate

### Q6: Query Latency at Scale
**Status**: Target is <10ms p95, not validated

**Questions**:
- Measured latencies with various query complexity?
- Cache hit rates in production?
- Slow query patterns (regex, fuzzy, aggregations)?
- p50/p95/p99 latency distribution?
- Query timeout handling?

**Impact**: Search responsiveness in Cortex UI

## Deployment & Operations

### Q7: Docker Image Optimization
**Status**: 2-3x larger than Meilisearch, needs reduction

**Questions**:
- Root cause of image size (Rust binary? Dependencies?)?
- Startup time vs Meilisearch?
- Memory footprint baseline (empty server)?
- Disk space consumed by empty indices?

**Impact**: Kubernetes resource costs, dev startup time

### Q8: Kubernetes Production Readiness
**Status**: Helm chart 75% complete

**Questions**:
- StatefulSet vs Deployment? (Currently unclear)
- PersistentVolume auto-provisioning?
- Init container for index bootstrap?
- Rolling update strategy for cluster?
- Resource request/limit recommendations per node size?
- Network policy for inter-node communication?

**Impact**: Cortex Kubernetes deployment guidance

### Q9: Backup & Disaster Recovery
**Status**: Snapshot API exists, recovery not tested at scale

**Questions**:
- Backup/restore time for 100GB index?
- Incremental backup deduplication?
- Cross-region replication overhead?
- Point-in-time recovery RPO (recovery point objective)?
- Backup retention policies and pruning?

**Impact**: SLA commitments for Cortex

## Integration & Compatibility

### Q10: MCP Protocol Maturity
**Status**: Designed but implementation status unclear

**Questions**:
- Is MCP integration actually implemented or future?
- Search vs retrieve vs aggregate operation semantics?
- Streaming result handling over MCP?
- Error propagation and recovery?
- Compatibility with Claude MCP spec versions?

**Impact**: Cortex brain integration capability

### Q11: Meilisearch API Compatibility
**Status**: Intentional divergence (LQL instead of JSON DSL)

**Questions**:
- Is Meilisearch compatibility a goal or non-goal?
- What breaks migration from Meili → Lexum?
- Client library compatibility (Python, JavaScript)?
- Query semantic differences (ranking, filtering)?

**Impact**: Migration effort from Meilisearch

### Q12: External Systems Integration
**Status**: Not clearly documented

**Questions**:
- How to integrate with Nexus external node IDs?
- Elasticsearch plugin ecosystem compatibility?
- Logstash/Beats integration?
- Grafana/Prometheus integration maturity?

**Impact**: Cortex operational monitoring setup

## Security & Compliance

### Q13: Authentication & Authorization Maturity
**Status**: API key + Bearer, but OAuth/mTLS incomplete

**Questions**:
- OAuth 2.0 implementation status?
- mTLS certificate management (auto-renewal)?
- SAML/LDAP support planned?
- Document-level security granularity?
- Audit log completeness and format?

**Impact**: Enterprise security requirements for Cortex

### Q14: Encryption at Rest
**Status**: Not yet implemented

**Questions**:
- Planned encryption method (LUKS, dm-crypt, built-in)?
- Key management strategy?
- Performance overhead of encryption?
- Index migration encrypted ↔ unencrypted?

**Impact**: Data protection compliance (GDPR, HIPAA)

### Q15: Data Retention & Compliance
**Status**: Not addressed

**Questions**:
- How to delete documents per user request (GDPR)?
- Snapshot retention policies?
- Audit trail retention?
- Secure deletion (not just tombstone)?

**Impact**: Privacy-sensitive Cortex deployments

## Testing & Reliability

### Q16: Chaos Engineering & Failure Modes
**Status**: Not documented

**Questions**:
- Tested failure modes (node down, network partition, disk full)?
- Recovery time from various failures?
- Data loss scenarios and prevention?
- Connection timeout behavior?
- Memory leak testing?

**Impact**: Production reliability of Cortex

### Q17: Load Testing Framework
**Status**: Infrastructure exists but results not shared

**Questions**:
- Available load test suites?
- Benchmark methodology?
- Published benchmark results vs competitors?
- Realistic workload profiles?

**Impact**: Capacity planning for Cortex

## Documentation & Knowledge

### Q18: Production Runbooks
**Status**: Operations guide exists, runbooks missing

**Questions**:
- Step-by-step playbooks for common failures?
- On-call diagnosis procedure?
- Escalation paths for different issue types?
- Post-incident review templates?

**Impact**: Cortex SRE operational procedures

### Q19: SDK Availability
**Status**: Planned Phase 2+ feature

**Questions**:
- Official Rust SDK implemented? (Cortex native)
- Python SDK available? (Cortex scripts)
- JavaScript SDK for frontend? (Cortex UI)
- SDK API stability and breaking change policy?

**Impact**: Cortex integration ease

### Q20: Community & Support
**Status**: GitHub issues/discussions available

**Questions**:
- Response time for bug reports?
- Security vulnerability disclosure process?
- Sponsorship/consulting available?
- Enterprise support SLA?

**Impact**: Cortex team support access

## Roadmap & Timeline

### Q21: Phase 2 Actual Timeline
**Status**: Planned "Q4 2025" but unclear

**Questions**:
- Firm commit date for distributed clustering?
- Known blockers or dependencies?
- Resource allocation to Phase 2?
- Risk of further delays?

**Impact**: Planning Cortex migration

### Q22: Phase 3 & 4 Sequence
**Status**: Vague descriptions, no detailed specs

**Questions**:
- Which comes first: Electron GUI or multi-protocol?
- Advanced aggregations priority?
- Performance tuning vs feature completeness?
- When considered "GA" (generally available)?

**Impact**: Long-term Cortex roadmap alignment

## Known Limitations to Document

1. **WSL Compatibility**: Tantivy issues with WSL 9p protocol (Windows native required)
2. **Alpha Status**: API may change, not recommended for critical production
3. **Distributed Not Ready**: Phase 2 in progress, clustering still 10% done
4. **No Vector Search**: Cannot yet support semantic search use cases
5. **Smaller Ecosystem**: Fewer plugins, tools, and community resources than Elasticsearch

## Recommended Research Priorities

For Cortex team considering Lexum adoption:

1. **Run load test** with Cortex document patterns (17 repos worth)
2. **Test Meili → Lexum migration** with actual Cortex data
3. **Measure search quality** (BM25 vs Meili relevance)
4. **Monitor Phase 2 progress** (distributed, production proof)
5. **Prototype MCP integration** with Cortex brain
6. **Profile Docker image** and suggest optimizations to Lexum team
7. **Benchmark Kubernetes deployment** (startup, resource usage)

## Summary

Lexum is **promising but immature**. Suitable for Cortex only after:
- Phase 2 completion (distributed features proven)
- 6+ months production deployment by other users
- Performance validation against Cortex workloads
- Operational playbooks and support channels mature
