# Nexus: Decisions & Rationale

## Phase 9: External Node IDs (Design Principles)

### Decision: Caller-Supplied IDs via `_id` Property

**Context**: HiveLLM ingestion pipelines need deterministic, idempotent node identity across re-imports (Cortex use case: file hash → stable node).

**Options considered**:
1. Expose internal IDs (u64) to callers — breaks encapsulation, leaks storage details
2. Side table mapping (external → internal) — requires separate synchronization
3. **Chosen**: Reserved `_id` property in Cypher with dedicated index

**Rationale**:
- Transparent to Cypher users; works in MATCH, CREATE, MERGE
- Deterministic from import tool perspective (hash-based external IDs)
- No additional network round-trips (query by _id returns in single traversal)
- Scales: O(log n) lookup via LMDB B-tree (both forward + reverse)

### Decision: 4 External ID Formats

**Formats**: Hash (Blake3/SHA-256/SHA-512), UUID, String, Bytes

**Why not 1 universal format?**
- **Hash**: content-addressable (Cortex: file SHA256)
- **UUID**: distributed generation (cross-system correlation)
- **String**: human-readable (natural keys: SKU, user handle, URN)
- **Bytes**: opaque binary (protocol buffers, binary serialization)

**Wire encoding**: Prefix scheme (`'sha256:...'`, `'uuid:...'`, `'str:...'`, `'bytes:...'`) for clarity.

### Decision: 3 Conflict Policies

**ERROR**: Fail on duplicate (validating create)
- Intent: catch accidental duplicate ingestion
- Use: one-time imports where collision = programmer error

**MATCH**: Return existing node (idempotent import)
- Intent: same batch applied multiple times → safe
- Use: Cortex re-indexing (day 1 files + day 2 files with duplicates)
- Behavior: properties from payload discarded; existing node unchanged

**REPLACE**: Update properties, preserve identity
- Intent: resynchronization with new values
- Use: version-sensitive config updates
- Behavior: properties overwritten; internal ID + labels stable

**Why not 1 mode?**
- Different workflows need different semantics
- ERROR prevents data loss from duplicate keys
- MATCH enables idempotent batch pipelines
- REPLACE enables version-driven updates

### Design Validation (Phase 10)

**Approach**: Live validation across all 6 SDKs on tagged Docker image

**Testing matrix**:
- Cypher surface (CREATE/MERGE/MATCH with _id)
- REST API (/data/nodes POST + GET)
- RPC wire (NodeOp surface)
- All conflict policies (ERROR, MATCH, REPLACE)
- All format variants (Hash, UUID, String, Bytes)
- All SDK languages (Rust, Python, TypeScript, Go, C#, PHP)

**Result**: 87/87 test cases passing on `hivehub/nexus:2.1.0` image (2026-04-30)

**Artifacts**: Cortex smoke test at `crates/cortex-workers/tests/nexus_external_id_smoke_it.rs`

## Phase 10: Multi-SDK Live Validation (Process)

### Decision: Validate on Running Container (Not Mocks)

**Context**: External ID feature ships in 6 SDKs simultaneously; coordination critical.

**Options**:
1. Unit tests per SDK (mocked server) — misses wire-protocol bugs
2. **Chosen**: Live IT against tagged Docker image

**Why**:
- Catches serialization bugs (REST/RPC/wire format)
- Validates all transports simultaneously
- Documents actual behavior (tests are specs)
- Enables quick rollback if a single SDK fails

### Decision: Smoke Test in Cortex (Not Nexus Repo)

**Location**: `crates/cortex-workers/tests/nexus_external_id_smoke_it.rs`

**Rationale**:
- Cortex is the primary Nexus consumer in HiveLLM
- Smoke test validates real use case (file-hash ingestion)
- Decouples Cortex testing from Nexus repo churn
- Enables parallel SDK fixes (Cortex test runner can retry)

**Test structure**:
```rust
// 1. Start nexus-server (Docker container or binary)
// 2. Create files with SHA256 external IDs
// 3. Verify idempotent re-import (MATCH policy)
// 4. Query by external ID
// 5. Assert properties unchanged on duplicate
```

**Gate**: `CORTEX_NEXUS_EXTERNAL_ID_IT=1` environment variable (opt-in, default off)

## Phase 9 + 10 Architectural Decisions

### Storage Layer: Two LMDB Sub-DBs

**Decision**: Separate forward (ExternalId → u64) and reverse (u64 → ExternalId) indexes

**Alternatives**:
1. Single index with bidirectional lookup code — error-prone
2. One-way index; reconstruct reverse on demand — slower
3. **Chosen**: Dual indexes, maintained atomically

**Why**:
- Fast both directions (O(log n) in each)
- Crash-safe: LMDB transactions atomic across both sub-DBs
- WAL replay: both updated together on node create/delete
- Backpressure from delete: reverse index used to clean up forward entry

### Executor: Conflict Checking at Storage Layer

**Decision**: ExternalId uniqueness enforced in `Storage::create_node()`, not Cypher layer

**Rationale**:
- Prevents TOCTOU (time-of-check time-of-use) race between planner + executor
- Single-writer model guarantees consistency
- Surfaces errors early (executor returns error; Cypher propagates)

### Cypher Semantics: `ON CONFLICT` Parity with Neo4j MERGE

**Decision**: Map `ON CONFLICT` to Neo4j's `ON MATCH SET` / `ON CREATE SET`

**Neo4j precedent**:
```cypher
MERGE (n:User {id: 'uuid:...'})
ON CREATE SET n.created = timestamp()
ON MATCH SET n.updated = timestamp()
```

**Nexus extension**:
```cypher
CREATE (n:Node {_id: 'uuid:...', data: 'value'}) ON CONFLICT MATCH
-- Equivalent to: MERGE (n {_id: ...}) ON CREATE SET ... ON MATCH RETURN n
```

## Rationales: Performance & Scalability

### Design: Record Stores (Fixed 32-byte Nodes, 48-byte Rels)

**Why fixed sizes?**
- Predictable memory access patterns (cache-friendly)
- O(1) seek to any node (offset = record_id * 32)
- No garbage collection (append-only WAL)
- Scales to billions of nodes (contiguous mmap)

### Design: Binary RPC Default (Not HTTP)

**Why MessagePack over JSON?**
- 40–60% smaller payloads
- 3–10× lower latency
- Bytes-native embeddings (NexusValue::Bytes)
- Native to Rust SDK (no serde_json overhead)

### Design: Epoch-Based MVCC (Not Lock-Based)

**Why epochs over row locks?**
- Readers never block writers
- Single-writer simplifies consistency
- Low GC overhead (epoch snapshots implicit)
- Scales to high read concurrency

### Design: HNSW KNN Per-Label (Not Global)

**Why per-label?**
- Separates embedding spaces (File vs Code vs Document)
- Enables label-specific recall tuning (M, ef parameters)
- Natural to graph semantics (nodes of same label likely similar)
- Cheaper than global KNN rebuild on label schema changes

## Cortex Relevance

### External ID Strategy for Cortex

**Chosen: SHA256(file_path + content)**

**Rationale**:
- Content-addressed: identical files across re-runs → same node ID
- Deterministic: no timestamps, UUIDs, or sequences
- Idempotent re-import: cortex-indexer can re-run day 1 job safely
- Disaster recovery: re-import from source repos reproduces exact graph

**Alternative (rejected): UUID**
- Non-deterministic: same file re-indexed → different node each time
- Requires side table for de-duplication
- Breaks idempotent pipeline invariant

### Conflict Policy for Cortex

**Chosen: MATCH**

**Rationale**:
- Cortex re-indexing is incremental (new files + duplicates)
- MATCH returns existing node; new properties discarded (safe)
- Idempotent semantics: batch job can be re-run without cleanup
- Example: `nexus-cortex-ingest --conflict=match files.csv` (day 1 + day 2 safe)

**Alternative (rejected): REPLACE**
- Could update stale file metadata on re-index
- But requires careful property merging (what if size changed?)
- MATCH + explicit UPDATE query safer (separation of concerns)

## Trade-Offs: Current Limitations

### Single-Writer MVCC
- **Trade-off**: Serializes writes; read-heavy optimal
- **For Cortex**: Acceptable (reads dominate; periodic batch ingestion)
- **Future**: Sharding (V2+) distributes writes across shards

### External IDs Per-Database (Not Global)
- **Trade-off**: External ID `'uuid:xyz'` can exist in multiple databases
- **For Cortex**: Use one database per environment (dev, staging, prod)
- **Future**: Global external ID pool if cross-DB references needed

### HNSW Recall Tuning
- **Trade-off**: M, ef parameters require workload profiling
- **For Cortex**: Supply tools for parameter sweep (phase11l onward)
- **Future**: Adaptive tuning based on query latency targets
