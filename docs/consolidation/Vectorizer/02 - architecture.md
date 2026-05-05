# Vectorizer — Architecture

**Last Updated:** 2026-05-04

## Engine Design

### Layered Stack

```
┌──────────────────────────────────────────────┐
│ Transport (HTTP, RPC, gRPC, MCP, GraphQL)    │ vectorizer-server
├──────────────────────────────────────────────┤
│ API Handlers (collections, search, graph)    │ vectorizer
├──────────────────────────────────────────────┤
│ Vector Store (collections, cache, WAL)       │ vectorizer
├──────────────────────────────────────────────┤
│ Embedding (TF-IDF, BM25, BERT, MiniLM)       │ vectorizer
├──────────────────────────────────────────────┤
│ Storage (HNSW, quantization, persistence)    │ vectorizer-core
├──────────────────────────────────────────────┤
│ Codec (MessagePack, bincode, compression)    │ vectorizer-core
└──────────────────────────────────────────────┘
```

## Core Modules

### 1. Storage & Persistence (`vectorizer-core` + `vectorizer`)

**HNSW Indexing:**
- Approximate nearest-neighbor search via Hierarchical Navigable Small-World graphs
- Configurable M (max neighbors) and ef_construction (insertion accuracy)
- In-memory with optional memory-mapping for datasets > RAM
- Sub-millisecond search on 10M+ vector datasets

**Quantization:**
- **Product Quantization (PQ):** 64x memory reduction, minimal accuracy loss
- **Scalar Quantization (SQ):** float32 → int8/int16 with configurable ranges
- Codebook learning + efficient distance computation

**Compression:**
- **LZ4:** fast streaming (default for snapshots)
- **Zstd:** high ratio (12-16 levels configurable)
- Per-collection codec selection at creation time

**Unified Storage Format (`.vecdb`):**
- Binary container with metadata header
- Vectorized payloads (JSON + typed scalars)
- HNSW graph adjacency lists
- Quantization codebooks
- Atomic snapshots with checksums

### 2. Embedding System (`vectorizer`)

**Sparse Methods:**
- **TF-IDF:** traditional baseline, variable dimensionality
- **BM25:** probabilistic ranking (k1=1.5, b=0.75), superior over TF-IDF

**Dense Methods:**
- **BERT (768D):** contextual semantic embeddings
- **MiniLM (384D):** efficient sentence embeddings
- **SVD-Reduced:** dimensionality reduction of sparse vectors (300D, 768D)
- **Custom models:** pluggable via FastEmbed interface

**Built-in Providers:**
- TensorFlow / Candle-based inference (CPU)
- Optional quantized models (int8 weight packing)
- Per-collection embedding selection at creation

### 3. Collections & Caching

**Dynamic Collections:**
- Created via REST/RPC/MCP at runtime
- CRUD-enabled, persisted to disk
- Optional file-watcher auto-indexing for workspace directories

**Workspace Collections (read-only):**
- Loaded from `workspace.yml` at boot
- Auto-rebuilt on config changes
- Cached in `.vecdb` format

**Memory Management:**
- Multi-tier cache (in-memory HNSW layer, memory-mapped payloads)
- Configurable cache sizes per collection
- HiveHub cluster mode enforces 1GB max total cache

### 4. Graph Relationships (`vectorizer`)

**In-Memory Per-Collection Graph:**
- Nodes (document IDs with payload snapshots)
- Typed edges (references, contains, derived_from, etc.)
- Edge weights for multi-hop traversal

**Discovery Pipeline:**
- Auto-detect edges from payload metadata (`references`, `file_path`)
- Similarity-based edge suggestion (threshold + ranking)
- Lazy opt-in (no graph until `enable_graph_for_collection`)

**Operations:**
- Single-hop neighbor traversal
- N-hop shortest path finding
- Edge creation/deletion (both explicit and auto-discovery)
- Relationship type filtering

Currently limited to CPU collections only (GPU/sharded rejected).

### 5. Search Pipeline

**Semantic Search:**
- Dense vector + HNSW index + similarity metrics (Cosine, Euclidean, Dot)
- Configurable ef_search (higher = more accurate, slower)
- Top-k ranking with optional reranking

**Hybrid Search:**
- BM25/TF-IDF sparse retrieval (fast candidate selection)
- HNSW dense re-ranking (semantic refinement)
- **Reciprocal Rank Fusion (RRF)** — fuses sparse + dense scores
- Multi-collection search via remote RPC routing

**Intelligent Search:**
- Query expansion (semantic paraphrasing)
- Semantic reranking (cross-encoder)
- Payload filtering + compound queries

### 6. Persistence & Replication

**Write-Ahead Log (WAL):**
- Atomic insert/update/delete operations
- Daily rotation + archive
- Recovery on restart

**Snapshots:**
- Full collection snapshot (all vectors + index + metadata)
- Automatic on schedule (configurable)
- Manual via API

**Replication (High Availability):**
- **Raft consensus** (openraft 0.10.0-alpha.17) — automatic leader election in 1-5s
- **Master-Replica** TCP streaming — exponential backoff on disconnect (5s-60s)
- Distributed sharding with automatic routing (horizontal scaling)

## Transport & Protocol

### VectorizerRPC (Binary, Default)

Frame format: `u32 length (LE) + MessagePack body (max 64 MiB)`

```
Request:  {id: u32, command: string, args: Vec<Value>}
Response: {id: u32, result: Result<Value, Error>}
```

Multiplexed connection pool on port 15503. Clients track in-flight `id` and dispatch responses by correlation.

### REST API (HTTP/JSON)

Standard JSON over HTTP/HTTPS. Endpoints:
- `/collections` — CRUD
- `/vectors` — insert/search/delete
- `/search/hybrid` — RRF + filtering
- `/graph/*` — relationship ops
- `/auth/*` — key management + token introspection
- `/cluster/*` — Raft + replication (admin-only)
- `/mcp` — MCP protocol entry point
- `/graphql` — GraphQL introspection + queries
- `/dashboard/` — embedded React SPA (~26MB)

### MCP (AI IDE Integration)

StreamableHTTP (JSON-RPC 2.0 over HTTP/1.1 + HTTP/2). 31 registered tools:
- **Core (9):** collections, vectors, search
- **Advanced (4):** intelligent/semantic/hybrid/extra search
- **Discovery (7):** file operations, query expansion
- **Graph (8):** nodes, edges, path-finding
- **Maintenance (3):** cleanup, stats

## Security & Auth

**Authentication:**
- JWT (short-lived, issued via `/auth/login`)
- API keys (long-lived, per-collection scopes)
- RFC 7662 token introspection
- Hardened session cookies (`HttpOnly; Secure; SameSite=Strict`)
- CSRF middleware on mutating requests

**Authorization:**
- Role-based access control (Admin, ReadWrite, ReadOnly)
- Per-API-key rate limiting with tiered overrides
- Scoped keys for collection-level permissions
- Audit log (in-memory ring + daily JSONL rotation)

**Encryption:**
- Payload encryption (optional ECC-P256 + AES-256-GCM)
- TLS 1.2/1.3 with mTLS support
- Configurable cipher suites

## Performance Characteristics

| Metric | Value |
|--------|-------|
| Search latency (CPU) | < 3ms |
| Search latency (Metal GPU) | < 1ms |
| Throughput | 4,400-6,000 QPS |
| 4-5x faster than Qdrant | 0.16-0.23ms vs 0.80-0.87ms |
| Storage savings | 20-30% (`.vecdb`) + 64x (PQ) |
| On-disk format | Unified binary `.vecdb` |

Benchmarks: `docs/specs/BENCHMARKING.md` · Qdrant comparison: `docs/specs/benchmarks/`.
