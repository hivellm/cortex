# Nexus: Cortex Relevance

## What Cortex Should Ingest from Nexus

Cortex acts as the **indexing + search backbone** for HiveLLM source analysis. Nexus provides the **persistent graph storage layer** for that indexed metadata. Below is the intake checklist: what Cortex should adopt from Nexus, categorized by artifact type and redaction scope.

## Source Code Integration Points

### Critical: `cortex-graph` Module
- **Path**: `crates/cortex-graph/` (Cortex repo)
- **Dependency**: `nexus-graph-sdk = "2.1.0"` (Rust SDK pinned in `Cargo.toml`)
- **Intake**: 
  - SDK's `create_node_with_external_id(label, properties, external_id, conflict_policy)` surface
  - `get_node_by_external_id(external_id)` for lookups
  - Binary RPC transport (default; HTTP fallback)
  - Error handling: `ExternalIdConflict`, connection failures, timeouts
- **Redaction**: None — production code, no secrets inlined

### High Priority: Schema Mapping
- **Path**: Nexus `docs/compatibility/cypher-schema.md` + `sdks/rust/examples/`
- **Intake**:
  - Label taxonomy: `Repository`, `File`, `Function`, `Class`, `Import`, `Call`
  - Property types (integer, string, list, map) for Cortex graph model
  - Relationship types: `CONTAINS`, `IMPORTS`, `CALLS`, `DEFINES`, `INHERITS`, `ANNOTATES`
  - External ID format: `sha256:...` for file content hashing (idempotent re-import)
- **Redaction**: Example payloads sanitized; no real codebase paths in schema docs

### External ID Strategy
- **Path**: Nexus phase 9 executor (`crates/nexus-core/src/executor/external_ids.rs`)
- **Intake**:
  - Conflict policies: `ERROR` (validate no dups), `MATCH` (idempotent batch), `REPLACE` (sync updates)
  - Storage: LMDB forward + reverse indexes (O(log n) lookup)
  - Formats: Hash (Blake3, SHA-256, SHA-512), UUID, String, Bytes
- **Cortex choice**: `sha256:<hash>` for file-level nodes (content-addressed identity)
- **Redaction**: Implementation details only; no test data

### Optional: Vector Index Configuration
- **Path**: Nexus `crates/nexus-core/src/index/hnsw.rs` + docs
- **Intake**:
  - HNSW parameters: `M` (degree, default 16), `ef_construct` (build-time), `ef_search` (query-time)
  - Metrics: cosine (default), L2, dot product
  - Per-label indexes (e.g., separate KNN pool for `File` vs `Code` vs `Document`)
- **Redaction**: Tuning recommendations for Cortex workload profiling

## Documentation & Specs

### Phase 9 & 10 External ID Design
- **Path**: Nexus `docs/decisions/phase9-external-ids.md` + `phase10-sdk-validation.md`
- **Intake**:
  - Design rationale: why caller-supplied IDs improve idempotency
  - Conflict policy semantics: when to use ERROR vs MATCH vs REPLACE
  - Format choice (hash-based for files)
- **Use in Cortex**: Inform cortex-graph's ingestion strategy

### Cypher Language Reference
- **Path**: Nexus `docs/cypher-reference.md` (300/300 Neo4j compat)
- **Intake**:
  - MATCH, CREATE, MERGE, SET, RETURN clauses (Cortex queries read-heavy)
  - Pattern matching: variable-length paths `*`, `*m..n`, `+`, `?`
  - Aggregation: COUNT, COLLECT, GROUP BY
  - List/map comprehensions for result transformation
- **Redaction**: None — canonical language spec

### REST API Surface
- **Path**: Nexus `docs/api/rest.md` + source `crates/nexus-server/src/http/`
- **Intake**:
  - POST `/cypher` for query execution with parameters
  - POST `/data/nodes` (phase 10) for external-ID node creation
  - GET `/data/nodes/by-external-id` for lookups
  - GET `/health`, `/prometheus` for operational checks
- **Redaction**: None — public API

## Architecture & Rationale

### MVCC + Single-Writer Model
- **Path**: Nexus `crates/nexus-core/src/transaction/` + docs/design
- **Intake**:
  - How Nexus guarantees read consistency (epoch-based snapshots)
  - Why single-writer simplifies external ID uniqueness enforcement
  - Implications for Cortex: write-heavy re-indexing must batch → queue jobs, not parallel writes
- **Rationale for Cortex**: Cortex ingestion jobs are append-dominant (batch files); Nexus MVCC makes reads fast (suitable for Cortex's query volume)
- **Redaction**: None — architectural overview

### Storage Layer: Record Stores + Catalog
- **Path**: Nexus `crates/nexus-core/src/storage/` + docs
- **Intake**:
  - nodes.store (32B fixed-size records): predictable seek time
  - rels.store (48B): doubly-linked adjacency for O(1) traversal
  - Catalog (LMDB): label/type/key ID mappings + external ID indexes
  - WAL (write-ahead log) for durability
- **Relevance**: Informs Cortex on query performance expectations (cache-friendly layouts)
- **Redaction**: None — architecture docs

## Learnings & Change History

### Phase 9 Learnings
- External ID formats (4 variants) outperform UUID-only approach (enables content-addressable + human-readable keys)
- Dual LMDB indexes (forward + reverse) necessary for crash-safe atomicity
- Conflict policies prevent common ingestion bugs (duplicate prevention + idempotent re-import)

### Phase 10 Learnings
- Live SDK validation (on running Docker container) catches wire-protocol bugs mocks miss
- All six SDKs shipping simultaneously requires coordinated testing (Cortex smoke test validates Rust SDK integration)

### Change History (Cortex-relevant)
- **2026-04-30**: Nexus 2.1.0 ships; phase 9 (external IDs) + phase 10 (SDK validation) complete
- **2026-05-02**: Cortex phase11l prerequisite gate met; SDK pinned to 2.1.0+
- **2026-05-04**: Nexus 2.2.0 released; sharded cluster core complete (optional future enhancement for Cortex scaling)

## Redaction Notes

**Include**:
- Source code paths (crate/module names)
- API signatures and type definitions
- Architecture diagrams and data flow
- Decision rationale (why design X was chosen)
- Performance characteristics (latency, throughput)

**Exclude**:
- Hardcoded credentials or API keys
- Test data with real codebase examples (use synthetic examples)
- Internal debugging logs or stack traces
- Customer-specific deployment details (use generic Helm/Docker examples)
- Uncommitted work or experimental branches

## Intake Checklist (Cortex Team)

- [ ] Read Nexus overview (01) + architecture (02)
- [ ] Study external ID design (04-data + 06-decisions sections)
- [ ] Review cortex-graph integration points (05 + 02)
- [ ] Validate smoke test (crates/cortex-workers/tests/nexus_external_id_smoke_it.rs)
- [ ] Pin SDK version in cortex-graph Cargo.toml
- [ ] Configure Nexus deployment (docker-compose or Helm for dev)
- [ ] Profile HNSW parameters for Cortex workload (if KNN indexes needed)
- [ ] Document Cortex's external ID strategy (file SHA256 + MATCH conflict policy)
