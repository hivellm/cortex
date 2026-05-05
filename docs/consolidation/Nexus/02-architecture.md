# Nexus: Architecture

## System Layers

```
┌─────────────────────────────────────────┐
│     Client Transports (15474-15476)     │
│  RPC | HTTP/JSON | RESP3                │
└──────────────────┬──────────────────────┘
         │
┌────────┴─────────────────────────────────┐
│     Cypher Executor & Query Planner      │
│  Parser → Planner → Operators → Results  │
└──────────────────┬──────────────────────┘
         │
┌────────┴─────────────────────────────────┐
│    Transaction Layer (MVCC + Locking)    │
│  Epoch-based snapshots, single-writer    │
└──────────────────┬──────────────────────┘
         │
┌────────┴─────────────────────────────────┐
│         Index Layer (4 index types)      │
│  Label Bitmap | B-tree | FTS | HNSW      │
└──────────────────┬──────────────────────┘
         │
┌────────┴─────────────────────────────────┐
│          Storage Layer                   │
│  Catalog (LMDB) | WAL | Record Stores    │
│  Page Cache | String Dictionary          │
└─────────────────────────────────────────┘
```

## Query Engine

### Cypher Executor
- **Parser** — linear-time O(N) grammar (from quadratic O(N²) in v1)
- **Planner** — heuristic cost-based with hint support (USING INDEX, USING SCAN, USING JOIN)
- **Operators** — pattern match, expand (BFS-optimized for bounded paths), filter, project, aggregation
- **Optimization** — JIT compilation via Cranelift on hot paths; runtime SIMD dispatch (AVX-512/AVX2/NEON)

### Pattern Matching
- Label intersection via RoaringBitmap (64-bit label support per node)
- Variable-length paths: `*`, `*n`, `*m..n`, `+`, `?` → optimized BFS
- Quantified path patterns: grammar + AST shipped (phase6 execution pending)

## Storage Architecture

### Record Stores (Neo4j-inspired)

**nodes.store** (32-byte records):
- `label_bits` (8B): bitmap of label IDs
- `first_rel_ptr` (8B): head of doubly-linked relationship list
- `prop_ptr` (8B): pointer to property chain
- `flags` (8B): deleted, locked, version bits

**rels.store** (48-byte records):
- `src_id`, `dst_id` (8B each): source/destination node IDs
- `type_id` (4B): relationship type
- `next_src_ptr`, `next_dst_ptr` (8B each): linked-list pointers for O(1) traversal
- `prop_ptr` (8B): property chain
- `flags` (4B): state bits

**props.store** (variable): Chain of property records (key_id, type, value, next_ptr)

**strings.store**: Deduplicated string/blob dictionary with CRC32 integrity

### Indexes (4 types)

1. **Label Bitmap** — RoaringBitmap per label for fast label scans
2. **B-tree** — composite indexes on node/relationship properties; supports UNIQUE flag
3. **Full-Text (Tantivy)** — per-index analyzer catalogue (standard, ngram, english, etc.), BM25 ranking, WAL auto-maintenance
4. **HNSW (KNN)** — per-label vector indexes; cosine, L2, dot metrics; bytes-native wire format

### Catalog (LMDB via heed)

Bidirectional ID mappings:
- label_name ↔ label_id
- type_name ↔ type_id
- key_name ↔ key_id
- Statistics: node count per label, rel count per type
- **External ID indexes**: Two sub-DBs (forward + reverse) for O(log n) external ID lookups

### WAL & Durability

- Append-only log with CRC32C (hardware-accelerated)
- Epoch-based MVCC snapshots
- Crash recovery: replay WAL on restart
- Replication stream: async/sync modes, master-replica, circular buffer (1M ops)

## Executor Branches (Phase 9 & 10)

### Phase 9: External Node IDs
- **Feature**: Caller-supplied stable identifiers via reserved `_id` property
- **Storage**: Two LMDB sub-DBs (forward + reverse index) in catalog
- **Formats**: Hash (Blake3/SHA-256/SHA-512), UUID, String (≤256B), Bytes (≤64B)
- **Conflict policies**: ERROR (fail if exists), MATCH (return existing), REPLACE (update properties)
- **Cypher surface**: `CREATE (n {_id: '...'}) ON CONFLICT ERROR|MATCH|REPLACE`, `MERGE`, `MATCH (n {_id: ...})`
- **REST API**: `/data/nodes` (POST with external_id + conflict_policy), `/data/nodes/by-external-id` (GET)

### Phase 10: Multi-SDK Live Validation
- **SDKs shipped**: All six SDKs (Rust, Python, TypeScript, Go, C#, PHP) at 2.1.0
- **SDK surface**: `create_node_with_external_id(label, properties, external_id, conflict_policy)`, `get_node_by_external_id(external_id)`
- **Validation**: 87/87 live SDK test cases on `hivehub/nexus:2.1.0` image (REST + RPC paths)
- **Intent**: Drop-in idempotent ingestion; deterministic re-import; cross-system joins

## Transaction Model

- **MVCC** via epoch-based snapshots
- **Single-writer** locking for consistency
- **Savepoints**: LIFO stack (SAVEPOINT, ROLLBACK TO SAVEPOINT, RELEASE SAVEPOINT)
- **Constraints enforced** on every CREATE/MERGE/SET/REMOVE/DELETE
- **Rate limiting** (three layers):
  - Per-key: 1k requests/min, 10k/hour
  - Per-connection: RPC semaphore (bounded concurrency)
  - Global: admission queue (FIFO + 503 Retry-After on timeout)

## Sharded Cluster (V2 Core Complete)

- **Hash partitioning**: xxh3 hash on node ID space
- **Per-shard Raft**: leader election ≤ 3× timeout (~900ms), majority quorum, log replication
- **Distributed coordinator**: scatter/gather with atomic failure semantics
- **Metadata**: generation-tagged to detect stale caches
- **Guarantees**: 3-replica shard tolerates 1 loss; 5-replica tolerates 2
- **Status**: Core complete (2026-04-20); multi-host TCP transport shipped; online re-sharding queued

## Performance Characteristics

- **KNN dot product**: 12.7× speedup @ dim=768 via SIMD (Zen 4)
- **Cypher parsing**: 290× faster on 32 KiB queries (O(N) vs O(N²))
- **Cache hit rates**: 90%+ on hierarchical L1/L2/L3 cache
- **vs Neo4j 5.15**: median 4.7× faster on representative workloads
