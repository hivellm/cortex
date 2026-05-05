# Lexum Data & Storage

## Index Structure

### Logical Organization
```
Index (global)
├── Shard 0 (primary)
│   ├── Segments (searchable units)
│   │   ├── Inverted Index (postings)
│   │   ├── Stored Fields (compressed docs)
│   │   ├── Fast Fields (column store)
│   │   └── Term Dictionary
│   ├── Meta (shard metadata)
│   └── WAL (write-ahead log)
├── Shard 1 (primary)
└── Shard N (replicas)
```

### Physical Storage
```
/data/lexum/
├── indices/
│   ├── index_1/
│   │   ├── shard_0/
│   │   │   ├── segments/
│   │   │   │   ├── segment_1.cfs (compound segment)
│   │   │   │   ├── segment_2.cfs
│   │   │   │   └── _0.del (deletion bitmap)
│   │   │   ├── meta.json
│   │   │   └── wal/ (transaction log)
│   │   └── shard_1/
│   └── index_2/
├── metadata/
│   └── rocksdb/ (cluster state, index metadata)
└── temp/ (temporary files)
```

## Schema Definition

### Field Types
- **text**: Full-text searchable, analyzed
- **keyword**: Exact match, not analyzed
- **i64**: 64-bit integer
- **f64**: 64-bit float
- **date**: ISO 8601 timestamp with range support
- **bool**: Boolean values
- **nested**: Complex objects with field relationships

### Schema Builder API
```rust
Schema::builder()
    .add_field("title", FieldOptions::text().store(true))
    .add_field("price", FieldOptions::i64())
    .add_field("category", FieldOptions::keyword())
    .add_field("created_at", FieldOptions::date())
    .build()
```

### Dynamic vs Strict Modes
- **Dynamic** (default): New fields automatically added
- **Strict**: Only pre-defined fields accepted

## Indexing Strategy

### Tantivy Inverted Index
- **Postings**: Lists of documents containing each term
- **Term Dictionary**: Fast term lookup and enumeration
- **Segment-based**: Multiple segments merged periodically
- **BM25 Scoring**: Probabilistic relevance ranking

### Segment Merging
- Configurable merge policy (logarithmic growth)
- Background compaction reduces segment count
- Read optimization through segment consolidation
- Write amplification trade-off manageable

### Document Analysis Pipeline
1. **Tokenization**: Split text into terms
2. **Filtering**: Remove stop words, lowercase, stem
3. **Indexing**: Add terms to inverted index
4. **Storage**: Compress and store original document

## Query Execution

### Query Types
- **Match**: Full-text search (BM25 scoring)
- **Term**: Exact field match
- **Range**: Numeric/date ranges with bounds
- **Boolean**: AND, OR, NOT combinations
- **Fuzzy**: Approximate matching (Levenshtein)
- **Phrase**: Multi-word exact sequence with slop
- **Prefix**: Term prefix matching
- **Wildcard**: Glob-style patterns

### Query Cache
- Caches compiled query plans
- LRU eviction with configurable size
- Thread-safe (DashMap-based)
- Saves parser/optimizer cycles

## Aggregations Framework

### Types
- **Terms**: Group by field values
- **Stats**: Min, max, avg, sum, count
- **Histogram**: Bucketed numeric ranges
- **Date Histogram**: Time-based bucketing
- **Nested**: Complex aggregations on nested objects
- **Pipeline**: Post-aggregation transformations

### Storage
- Field cache (column store) for fast aggregation
- In-memory bucketing during execution
- Sorted results by bucket key

## Backup & Snapshots

### Snapshot System
- Point-in-time backup of indices
- Repository management (filesystem, S3, etc.)
- Incremental backups with delta storage
- Metadata stored in RocksDB

### Restore Operations
- Full index restore from snapshot
- Selective shard restore
- Cross-cluster restore capability
- Snapshot versioning and management

## Metadata Storage (RocksDB)

### Stored Data
- Cluster state (nodes, assignments, health)
- Index metadata (schema, settings, statistics)
- User data (credentials, API keys, permissions)
- Configuration state
- Shard assignments and replication info

### Persistence
- LSM tree (Log-Structured Merge) structure
- Synchronous writes for durability
- Compaction reduces write amplification
- Snapshots for atomic backup

## Performance Characteristics

### Write Performance
- **Indexing Throughput**: 50K-100K docs/sec target
- **Latency**: <100ms for single document
- **Batch Ops**: Optimized for bulk ingestion

### Read Performance
- **Search Latency**: <10ms p95 (target)
- **Query Cache**: Reduces repeated query cost
- **Field Cache**: Sub-millisecond aggregations
- **Throughput**: 10K+ queries/sec per node

### Scalability Limits
- Max shard size: Unlimited (distributed)
- Max document size: 100MB (configurable)
- Max index count: Limited by disk/RAM
- Max query size: 10MB
