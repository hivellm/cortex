# Vectorizer — Data & Storage

**Last Updated:** 2026-05-04

## Collection Schema

### Vector Payload

Each vector has:
- **id** (string, unique per collection) — user-provided or auto-generated UUID
- **vector** (float array, dimension D) — the dense embedding
- **payload** (JSON object) — structured metadata
  - `file_path` (promoted as first-class index key)
  - `text` (original source, for sparse indexing)
  - `timestamp` (creation or modification time)
  - Custom fields (user-defined, optional)
- **sparse_vector** (optional) — BM25/TF-IDF weights when hybrid search enabled

### Collection Metadata

```json
{
  "name": "my_docs",
  "dimension": 384,
  "embedding_type": "minilm",
  "distance_metric": "cosine",
  "quantization_config": {
    "method": "pq",
    "codebook_size": 256,
    "residual_bits": 8
  },
  "hnsw_config": {
    "m": 16,
    "ef_construction": 200,
    "ef_search": 40
  },
  "created_at": "2025-10-15T10:30:00Z",
  "vector_count": 150000,
  "deleted_vector_count": 523,
  "size_bytes": 45678321
}
```

## Storage Format (`.vecdb`)

Unified binary format replacing separate vector + index files.

### Structure

```
.vecdb file
├── Magic + version (8 bytes)
├── Metadata header (JSON + length)
├── Vector data (packed float32)
├── HNSW graph (adjacency lists + layer info)
├── Quantization codebooks (PQ)
├── Payload index (key-value + inverted text index)
├── Checksums (CRC32 per section)
└── Footer (offsets, lengths)
```

### File Organization

**On disk:**
```
data/
├── collections/
│   └── {collection_name}/
│       ├── vectors.vecdb         # Main storage (binary)
│       ├── wal.log                # Write-ahead log (JSON lines)
│       ├── snapshots/
│       │   └── {timestamp}.vecdb   # Full snapshot
│       └── metadata.json           # Collection metadata
└── graph/
    └── {collection_name}_graph.json  # Relationship edges (JSON)
```

### Advantages

- **20-30% space savings** (unified format vs separate vector + index + metadata files)
- **Atomic snapshots** — no partial writes
- **Memory-mapping** — efficient paging for datasets > RAM
- **Compression** — LZ4/Zstd compression per collection
- **Corruption detection** — CRC32 checksums per section

## Vector Storage

### In-Memory Layout

```
Collection {
  vectors: Vec<f32>,           // Packed [x₁, y₁, z₁, x₂, y₂, z₂, ...]
  hnsw_index: HNSWIndex,       // Graph layers + neighbor lists
  payloads: HashMap<ID, JSON>, // Metadata
  text_index: InvertedIndex,   // BM25/TF-IDF for sparse search
  quantization: Codebook,      // PQ or SQ parameters
  cache: LruCache,             // Hot-path reuse
}
```

### Vector ID Mapping

Vectors are stored in a flat array; logical IDs map to array indices via:
- `id_to_index: HashMap<String, u32>` (string ID → array offset)
- Deletion marks the index as tombstoned (retained for index stability)
- Periodic compaction removes tombstones (no in-place deletion)

## Indexing (HNSW)

### Graph Structure

```
Layer L (top):           [n₅]
                          │
Layer L-1:        [n₂]—[n₅]—[n₇]
                   │     │     │
Layer 0 (base):  [n₁]—[n₂]—[n₅]—[n₇]—[n₃]
                  │          │
                 [n₄]————————[n₆]
```

- Logarithmic level assignment (new node joins log N / ln 2 layers)
- M neighbors per layer (configurable, default 16)
- ef_construction (insertion parameter, default 200) — higher = more accurate, slower
- ef_search (query parameter, default 40) — higher = more results, slower

### Search Process

1. Enter at top layer (node with minimum distance to query)
2. Greedy nearest-neighbor search within layer (ef candidates)
3. Descend to next layer, repeat from nearest candidate
4. Return top-k neighbors from layer 0

**Latency:** < 3ms for 10M vectors on CPU (< 1ms on Metal GPU).

## Quantization

### Product Quantization (PQ)

**Purpose:** 64x memory reduction (float32 → int8)

**Process:**
1. Divide D-dimensional vector into M subspaces
2. Train codebook per subspace (k=256 centroids, k-means clustering)
3. Represent each dimension chunk as index into codebook (1 byte)
4. Store codebook + index array (compact residual representation)

**Trade-off:** ~1-2% accuracy loss vs 64x memory savings

### Scalar Quantization (SQ)

**Purpose:** 4x memory reduction (float32 → int8)

**Process:**
1. Map float range [min, max] → [0, 255]
2. Quantize all vectors to int8
3. Recompute distances in quantized space

**Trade-off:** ~3-5% accuracy loss vs 4x memory savings

## Persistence & Recovery

### Write-Ahead Log (WAL)

**File:** `wal.log` (JSON lines, one per operation)

```json
{"ts": "2025-10-15T10:30:00Z", "op": "insert", "id": "doc-123", "vector": [...]}
{"ts": "2025-10-15T10:30:01Z", "op": "update", "id": "doc-123", "payload": {...}}
{"ts": "2025-10-15T10:30:02Z", "op": "delete", "id": "doc-123"}
```

**Operations logged:**
- Insert vector
- Update payload / metadata
- Delete vector
- Create collection
- Delete collection

**Recovery:** On restart, replay WAL in order to reconstruct state.

**Rotation:** Daily (configurable), old logs archived.

### Snapshots

**Full snapshot:** Complete collection state (vectors + index + metadata)

**Scheduling:**
```yaml
snapshots:
  enabled: true
  interval: 3600  # Every hour
  compression: zstd
  level: 12       # 1-22 (higher = more ratio, slower)
```

**Manual trigger:**
```bash
curl -X POST http://localhost:15002/collections/my_docs/snapshot
```

**Restoration:** Load from snapshot on startup if WAL is corrupted.

## Cache Architecture

### Multi-Tier Strategy

```
L1 Cache (In-memory vectors + HNSW)
    ↓ (miss)
L2 Cache (MMap payloads)
    ↓ (miss)
L3 Disk (.vecdb file)
```

**L1 (Vectors):**
- All vectors in memory (after load)
- HNSW graph navigated in-memory
- Shared-state RwLock for concurrent search

**L2 (Payloads):**
- Optional memory-mapping of `.vecdb` payloads
- OS paging handles eviction
- Efficient for datasets > RAM

**L3 (Disk):**
- `.vecdb` file on SSD/NVMe
- Loaded on-demand during search if not cached

**Metrics:**
- Hit rate (L1, L2)
- Eviction count (LRU policy)
- Memory usage tracking

## Backup & Recovery

### Manual Backup

```bash
vectorizer backup create --collection my_docs --output backup.tar.gz
# Creates tar + gzip of .vecdb + WAL + metadata
```

### Automated Backup

```yaml
backup:
  enabled: true
  schedule: "0 2 * * *"      # 2 AM daily (cron syntax)
  retention_days: 30          # Keep last 30 days
  compression: true           # gzip
  incremental: true           # Delta-only after first full
  target: "s3://bucket/vectorizer/backups"  # Optional S3
```

### Restoration

```bash
vectorizer backup restore backup.tar.gz --collection my_docs
# Extracts .vecdb + WAL, replays WAL to current time
```

### Point-in-Time Recovery

WAL enables recovery to any past timestamp:
```bash
vectorizer recovery --collection my_docs --until "2025-10-15T09:00:00Z"
```

## Storage Optimization

### Size Estimates

| Configuration | Space per vector | 10M vectors |
|---------------|------------------|-------------|
| Uncompressed (384D, float32) | 1.5 KB | 15 GB |
| + HNSW index (M=16) | +0.3 KB | +3 GB |
| + Payloads (avg 500B JSON) | +0.5 KB | +5 GB |
| **Subtotal** | **2.3 KB** | **23 GB** |
| With PQ 64x | 0.04 KB | 0.4 GB |
| With SQ 4x | 0.4 KB | 4 GB |
| **With zstd L12** | **0.8 KB** | **8 GB** |

### Compression Best Practices

- **LZ4:** fast streaming, lower ratio (good for WAL)
- **Zstd L6-9:** balanced (good for snapshots, typical retention)
- **Zstd L12-16:** high ratio (good for archival, infrequent access)

## Data Migration (Qdrant → Vectorizer)

### Config Migration

Parse Qdrant YAML/JSON and generate Vectorizer config:
```bash
vectorizer migrate config --input qdrant.yml --output vectorizer.yml
```

### Data Export

```bash
qdrant-cli collection export --collection my_collection --output export.jsonl
```

### Data Import

```bash
vectorizer collection import --name my_collection --input export.jsonl --embedding minilm
```

### Validation

```bash
vectorizer migrate validate --source http://qdrant:6333 --target http://vectorizer:15002
```

Checks vector count, payload schemas, search result consistency.
