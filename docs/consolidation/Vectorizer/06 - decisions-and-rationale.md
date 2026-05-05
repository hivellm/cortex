# Vectorizer — Decisions & Design Rationale

**Last Updated:** 2026-05-04

## Architectural Decisions

### 1. HNSW over Alternatives

**Decision:** Implement HNSW (Hierarchical Navigable Small World) in-house rather than wrap Qdrant or pgvector.

**Rationale:**
- **Latency:** < 3ms on CPU, < 1ms on Metal GPU (vs Qdrant 1-5ms, pgvector 5-50ms)
- **Control:** In-process, no network roundtrip
- **Embedding co-location:** Vectors + metadata + index in single `.vecdb` file
- **Raft consensus:** Easier to wire replication without external coordination
- **Cost:** Single-binary deployment vs separate Qdrant cluster

**Trade-off:** Maintenance burden. Mitigated by using proven HNSW algorithm (Yandex/Qdrant compatible) + extensive benchmarking against reference implementations.

**References:**
- `docs/specs/PERFORMANCE.md` — latency benchmarks
- `crates/vectorizer/src/db/hnsw.rs` — implementation

### 2. VectorizerRPC as Default Transport

**Decision:** Make binary RPC (MessagePack over TCP) the default for SDKs, while keeping REST as universal fallback.

**Rationale:**
- **Bulk ingest:** Fire-and-forget pattern with configurable batching
- **Low latency:** u32 length + MessagePack body vs HTTP framing + JSON
- **Multiplexing:** One TCP connection, parallel in-flight requests
- **Determinism:** No JSON parsing overhead on hot path (search)

**REST preserved for:**
- Browser clients (CORS-friendly, fetch API native)
- Ops tooling (curl, httpie)
- Stateless HTTP-only environments
- Debugging (human-readable payloads)

**Alternative considered:** gRPC. Rejected due to protobuf codegen overhead per language.

**References:**
- `docs/specs/VECTORIZER_RPC.md` — wire specification
- `crates/vectorizer-protocol/` — MessagePack schema
- `docs/deployment/rpc.md` — operator guide

### 3. Unified `.vecdb` Storage Format

**Decision:** Single binary file (vectors + HNSW + payloads + quantization codebooks) instead of separate vector/index/metadata files.

**Rationale:**
- **Atomic snapshots:** No partial-write window
- **Space savings:** 20-30% (no duplicate index structures)
- **Memory mapping:** Efficient paging for datasets > RAM
- **Corruption detection:** CRC32 checksums per section
- **Backup simplicity:** One file to backup, no coordination

**Trade-off:** Seeking within large files (mitigated by memory-mapping on modern OSes).

**References:**
- `docs/specs/PERSISTENCE.md` — format specification
- `crates/vectorizer/src/db/persistence/` — serialization

### 4. Product Quantization for Memory Savings

**Decision:** Implement PQ (Product Quantization) as primary compression, not just SQ (Scalar Quantization).

**Rationale:**
- **64x memory reduction** (float32 → int8 per codebook)
- **Minimal accuracy loss** (< 2% vs baseline)
- **Fast distance:** Lookup table + few bitwise ops
- **Scalability:** 10M vectors in < 1GB (vs 15GB uncompressed)

**Trade-off:** Codebook training overhead at collection creation (offline, one-time).

**Adoption:** Optional per-collection configuration at creation time.

**References:**
- `docs/specs/PQ_IMPLEMENTATION.md` — implementation details
- `crates/vectorizer-core/src/quantization/` — codebook training

### 5. Raft for HA, Master-Replica for Scaling

**Decision:** Dual replication model: Raft (consensus leader) + Master-Replica (TCP streaming).

**Rationale:**
- **Raft:** Automatic failover in 1-5s, quorum safety
- **Master-Replica:** Full/partial sync with exponential backoff, simpler operational model
- **Flexibility:** Choose model per deployment (HA = Raft, Scaling = Master-Replica)
- **Sharding:** Distributed collections with automatic routing

**Trade-off:** Two replication code paths. Mitigated by shared WAL layer.

**Raft pinning:** openraft 0.10.0-alpha.17 (latest stable; 0.11+ has breaking changes not yet migrated).

**References:**
- `docs/specs/REPLICATION.md` — architecture
- `crates/vectorizer/src/db/replication/` — implementation

### 6. Graph as Lazy Opt-In Feature

**Decision:** Graph relationships are per-collection and must be explicitly enabled (not default).

**Rationale:**
- **No overhead for non-graph workloads** (memory, insertion latency)
- **Explicit intent:** User knows when traversal is needed
- **Separation of concerns:** HNSW is independent of graph topology
- **Current limitation:** CPU collections only (GPU/sharded rejected for now)

**Future:** GPU-aware graph, distributed graph across shards.

**References:**
- `docs/specs/GRAPH_RELATIONSHIPS.md` — specification
- `crates/vectorizer/src/db/graph.rs` — implementation

### 7. Built-in Embedding Models (No External ML Infra)

**Decision:** Embed TF-IDF, BM25, BERT, MiniLM in the binary, no separate model server.

**Rationale:**
- **Single binary deployment** (no model API coordination)
- **Fast inference** (CPU-only Candle models or FastEmbed)
- **Determinism:** Reproducible embeddings across restarts
- **Cost:** No separate GPU instance for embeddings

**Trade-off:** Large binary (~50MB gzipped with all models). Mitigated by feature-gating models (`candle-models`, `fast-embed`).

**Custom models:** Pluggable interface via `EmbeddingProvider` trait (users can inject custom logic).

**References:**
- `docs/specs/EMBEDDING.md` — model reference
- `crates/vectorizer/src/embedding/` — providers

### 8. Security: JWT + Scoped API Keys + RBAC

**Decision:** Multi-tier auth (short-lived JWT for interactive, long-lived keys for services).

**Rationale:**
- **JWT:** Dashboard sessions, short TTL (1 hour default)
- **API Keys:** Service-to-service, long TTL (90+ days configurable)
- **Scoped Keys:** Per-collection permissions (e.g., key1 can read cortex-decisions but not admin-panel)
- **RBAC:** Admin, ReadWrite, ReadOnly roles
- **Audit log:** Every authenticated operation logged to JSONL

**Hardened v3.3:**
- Session cookies: `HttpOnly; Secure; SameSite=Strict`
- CSRF middleware on mutating requests (automatic echo of token)
- Dev-mode auth bypass (loopback only, flags boot failure on non-loopback)

**References:**
- `docs/users/getting-started/DOCKER_AUTHENTICATION.md` — setup
- `SECURITY.md` — policy
- `docs/specs/API_REFERENCE.md` — endpoint reference

## Feature Evolution

### Hybrid Search (Dense + Sparse RRF)

**Rationale:** Neither pure dense (good recall, poor exact keyword match) nor pure sparse (good keyword, poor semantics) is sufficient.

- **Stage 1:** BM25 sparse retrieval (fast, top-50 candidates)
- **Stage 2:** HNSW dense re-ranking (semantic quality)
- **Fusion:** Reciprocal Rank Fusion (RRF) — combines ranks avoiding score normalization
- **Result:** 85% improvement in search quality over dense-only (v0.3.1 onward)

**Alternative considered:** Learning-to-rank (LTR). Rejected due to complexity + offline retraining burden.

### Graph Discovery & Relationship Types

**Rationale:** Explicit edge creation is error-prone; auto-discovery from metadata is more robust.

- **Metadata-driven:** Detect `references`, `file_path`, `contains` fields automatically
- **Similarity-based:** Suggest edges from high-similarity pairs (threshold-configurable)
- **Types:** `references`, `contains`, `derived_from`, `similar`, `parent`, `child`, custom

**Limitation:** CPU-only (GPU graph materialization would require distributed coordination).

### File Watcher & Workspace Collections

**Rationale:** Developers index source code; changes should propagate automatically.

- **Input:** `workspace.yml` with file paths + collection config
- **Change detection:** File creation/modification/deletion triggers re-indexing
- **Debouncing:** 300ms (configurable) to batch rapid changes
- **Read-only:** Workspace collections cannot be mutated via API

**Trade-off:** File system monitoring overhead on large projects. Mitigated by exclude patterns (`.git`, `node_modules`, `__pycache__`).

## Performance Optimizations

### SIMD Acceleration

**Decision:** Runtime CPU feature detection (AVX2, AVX-512, NEON, SVE, WASM) with fallback.

**Impact:** 5-10x faster vector ops (distance, quantization, normalization)

**Implementation:**
- Feature gates: `simd`, `simd-avx2`, `simd-avx512`, etc.
- Dynamic dispatch at runtime (no compile-time target-cpu requirement)
- Graceful fallback to scalar code

**References:** `crates/vectorizer-core/src/simd/` — dispatch and kernels

### Metal GPU (macOS Apple Silicon)

**Decision:** Optional Metal acceleration via `hive-gpu` crate (feature-gated, v0.2).

**Impact:** < 1ms search latency on Apple Silicon (vs < 3ms on CPU)

**Limitation:** macOS only (ARM64 architecture). Monitored for future Nvidia CUDA integration.

**Feature gate:** `hive-gpu` (default off)

### Sparse Index Caching

**Decision:** BM25 / TF-IDF vocabularies cached in memory.

**Impact:** Fast sparse retrieval without re-tokenization on every search

**Trade-off:** Memory overhead (typically < 50MB per large collection)

### In-Memory HNSW + Memory-Mapped Payloads

**Decision:** Vectors + graph in RAM, payloads on disk (MMap).

**Impact:**
- HNSW traversal stays on hot cache (fast)
- Payloads fetched on-demand (no memory bloat)
- Works for datasets > RAM

**Configuration:**
```yaml
memory:
  enable_mmap: true        # Memory-map payloads
  max_cache_memory_bytes: 4294967296  # 4GB for vectors
```

## Known Limitations & Future Work

### Current Limitations

1. **Graph relationships:** CPU collections only (GPU/sharded unsupported)
2. **Distributed sharding:** Basic support; distributed graph rebalancing WIP
3. **Model serving:** CPU-only embeddings (CUDA support planned)
4. **Qdrant API compatibility:** 95% (some snapshot + search-group edge cases)

### Future Roadmap

- **GPU graph:** Distributed graph traversal on sharded HNSW
- **CUDA embeddings:** GPU-accelerated inference for faster indexing
- **Columnar storage:** Apache Arrow for analytics workloads
- **Federated search:** Query across multiple Vectorizer instances (geo-distributed)
- **LSM-tree persistence:** Alternative to WAL for very high write rates

See: `docs/future/FUTURE_ROADMAP.md`

## Design Principles

1. **Performance first:** Latency < 3ms (CPU), < 1ms (GPU) is non-negotiable
2. **Self-contained:** Single binary, no external dependencies at runtime
3. **OpenAPI:** REST + RPC + gRPC parity (any client choice)
4. **Backward compatible:** API versioning; old clients work with new servers
5. **Operator-friendly:** Clear logging, metrics, recovery procedures
6. **Research-backed:** HNSW from Yandex, PQ from Facebook research, hybrid from Apache Lucene

## See Also

- `CHANGELOG.md` — release history and feature adoption
- `docs/migration/rpc-default.md` — v3.x transport migration
- `docs/specs/BENCHMARKING.md` — methodology and results
- `.rulebook/decisions/` — Cortex-specific architectural choices
