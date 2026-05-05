# Nexus: Data & Storage

## Graph Data Model

### Nodes
- **Labels** — up to 64 per node via bitmap (64-bit label_bits field)
- **Properties** — key-value pairs of typed values (integer, float, string, bytes, list, map, temporal)
- **Internal ID** — u64 physical record offset (immutable, used for all graph traversal)
- **External ID** — optional caller-supplied identifier (phase 9+)

### Relationships
- **Type** — single relationship type per edge
- **Properties** — key-value pairs, same types as nodes
- **Direction** — explicitly directed (src → dst)
- **Internal ID** — u64 record offset
- **Traversal** — O(1) via doubly-linked adjacency lists on both endpoints

### Property Types
- **Scalars**: null, boolean, integer (i64), float (f64), string (UTF-8)
- **Bytes** — opaque binary data (base64 wire format), ≤64 MiB per property
- **Collections**: LIST (homogeneous, typed), MAP (key-value)
- **Temporal**: date, datetime, time, duration, localdatetime

## Storage Layout

### Data Directory Structure
```
data/
├── catalog.mdb              # LMDB catalog (label/type/key mappings)
├── nodes.store              # 32-byte node records
├── rels.store               # 48-byte relationship records
├── props.store              # Variable-size property records
├── strings.store            # String/blob dictionary
├── wal.log                  # Write-ahead log (v1 + v2 frames)
├── checkpoints/             # Per-epoch snapshots
└── indexes/
    ├── label_*.bitmap       # RoaringBitmap per label
    ├── btree_*.idx          # B-tree indexes
    ├── fts_*.idx            # Tantivy full-text indexes
    └── hnsw_*.bin           # HNSW KNN indexes
```

### Catalog (LMDB)

Two LMDB instances:
1. **Main catalog**: Label/type/key ID mappings, statistics, schema metadata
2. **External ID catalogs** (phase 9):
   - `external_ids` sub-DB: ExternalId (encoded) → u64 internal ID
   - `internal_ids` sub-DB: u64 internal ID → ExternalId (encoded)

All LMDB operations use `heed` (version 0.20) with `read-txn-no-tls` feature for Windows TLS slot management.

## Indexes (4 types)

### 1. Label Bitmap (RoaringBitmap)
- One per label
- Fast label scans: `MATCH (n:Person)`
- Label intersection for multi-label queries
- Auto-maintained on CREATE/SET LABEL/REMOVE LABEL/DELETE

### 2. B-tree (Composite)
- Single-property and multi-property (composite) support
- Exact match, prefix seek, range seeks
- Optional uniqueness constraint flag
- Auto-maintained via WAL subscriber
- Types: integer keys, string keys, composite

### 3. Full-Text (Tantivy 0.22)
- Per-index analyzer catalogue: standard, whitespace, simple, keyword, ngram, english, spanish, portuguese, german, french
- BM25 ranking
- Query via `CALL db.index.fulltext.queryNodes(index_name, query)`
- Async writer with optional `refresh_ms` cadence
- WAL integration: auto-populate on CREATE/SET/REMOVE/DELETE with crash-recovery replay
- Throughput: >60k docs/sec bulk ingest; <5ms p95 single-term query

### 4. HNSW (Approximate KNN)
- Per-label vector indexes
- Metrics: cosine, L2, dot product
- Bytes-native embeddings on RPC wire (little-endian f32 array)
- Query: `CALL vector.knn(label, vector, k) YIELD node, score`
- Hybrid: combine with graph traversal in a single query

## External Node IDs (Phase 9)

### Motivation
- **Idempotency**: File-hash ingestion creates deterministic node identity
- **Disaster recovery**: Re-import same data reproduces exact graph topology
- **Cross-system joins**: System A and B reference same Nexus node without mapping table

### Formats (4 variants in ExternalId enum)

1. **Hash**
   - Blake3 (32 bytes): `'blake3:d7a8fbb307d7809469ca9abdcbed9e5104cc07ff76718b6491f745474949df5e'`
   - SHA-256 (32 bytes): `'sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'`
   - SHA-512 (64 bytes): `'sha512:cf83e1357eefb8bdf1542850d66d8007d620e4050...'`

2. **UUID**
   - RFC 4122 form: `'uuid:550e8400-e29b-41d4-a716-446655440000'`

3. **String**
   - UTF-8, ≤256 bytes: `'str:user-12345'`, `'str:sku-AB-001'`, `'str:service:auth:v2'`

4. **Bytes**
   - Opaque binary, ≤64 bytes

### Storage & Lookup

- **Catalog sub-DBs**: Forward (ExternalId → u64) + reverse (u64 → ExternalId)
- **Lookup time**: O(log n) via LMDB B-tree
- **Atomicity**: Both indexes updated together on node create/delete
- **WAL replay**: Safe across crash recovery; indexes rebuilt if needed
- **Scope**: Per-database (external IDs unique within one database, not globally)

### Conflict Policies (3 modes)

1. **ERROR** — fail if external ID already exists
   - Defensive create; validates no accidental duplicates
   - Returns `ExternalIdConflict` error

2. **MATCH** — return existing node unchanged
   - Idempotent batch import: same data applied multiple times creates once
   - New properties discarded if node exists
   - Typical: `nexus-ingest --conflict=match files.csv` (day 1 + day 2 safe)

3. **REPLACE** — update properties, preserve identity
   - Full re-sync with new values; internal node ID + labels unchanged
   - Properties overwritten from CREATE/MERGE payload
   - Typical: `CREATE (config) ON CONFLICT REPLACE` for versioned config

### Cypher Surface

```cypher
-- Create with external ID
CREATE (n:Node {_id: 'uuid:...', data: 'value'}) ON CONFLICT ERROR
CREATE (f:File {_id: 'sha256:...', path: '/file.txt'}) ON CONFLICT MATCH
CREATE (config:Config {_id: 'str:prod-config', version: 2}) ON CONFLICT REPLACE

-- Query by external ID
MATCH (n {_id: 'str:user-123'})
RETURN n.name

-- MERGE also uses external ID
MERGE (u:User {_id: 'uuid:550e8400-...'})
ON CREATE SET u.name = 'Alice'
```

### REST / SDK Surface (Phase 10)

**REST**: `POST /data/nodes`
```json
{
  "label": "Document",
  "properties": {"title": "Report"},
  "external_id": "uuid:550e8400-...",
  "conflict_policy": "MATCH"
}
```

**All SDKs**:
```rust
// Rust
client.create_node_with_external_id("Document", properties, external_id, ConflictPolicy::Match)

// Python
client.create_node_with_external_id("Document", properties, external_id, "match")

// TypeScript
client.createNodeWithExternalId("Document", properties, externalId, "match")
```

## Constraints & Validation

### Types
1. **UNIQUE** — enforced on every CREATE/MERGE/SET path
2. **NODE KEY** — composite uniqueness + implicit NOT NULL
3. **NOT NULL** — on nodes and relationships
4. **Property-type** — `IS :: INTEGER|FLOAT|STRING|BOOLEAN|BYTES|LIST|MAP` (strict, INTEGER ≠ FLOAT)

### Backfill & Enforcement
- Constraints validated on CREATE/MERGE/SET/REMOVE/DELETE
- Backfill validator: first 100 offending rows surfaced; atomic abort
- Schema DDL: `FOR (n:L) REQUIRE (...)` (Cypher 25)

## Transactional Semantics

- **MVCC** — epoch-based read snapshots; reads never block writers
- **Single-writer** — only one writer active at a time; readers concurrent with writer
- **Savepoints** — LIFO stack: `SAVEPOINT s1` → `ROLLBACK TO SAVEPOINT s1` → `RELEASE SAVEPOINT s1`
- **Durability** — WAL flushed before commit returns
- **Crash recovery** — WAL replayed on restart; all committed transactions restored

## Performance Tuning Knobs

- **Page cache size**: configurable per deployment
- **WAL flush mode**: sync (durable) vs async (faster, data-loss risk)
- **HNSW parameters**: M (connections), ef_construct, ef_search per index
- **FTS async writer**: `refresh_ms` cadence (default: sync read-your-writes)
- **Index batch size**: bulk ingest optimization
