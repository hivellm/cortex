# Vectorizer — Overview

**Status:** Production Ready (v3.3.0)  
**Language:** Rust 1.92+ (Edition 2024)  
**License:** Apache-2.0  
**Repository:** https://github.com/hivellm/vectorizer

## Purpose & Role

Vectorizer is a high-performance vector database and semantic search engine built in Rust. It serves as the core indexing and retrieval layer for HiveLLM's unified AI platform, enabling:

- **Semantic search** across vector embeddings (< 3ms CPU latency, < 1ms with Metal GPU)
- **Document processing** (14 formats: PDF, DOCX, XLSX, HTML, images, etc.)
- **Hybrid search** combining dense (HNSW) + sparse (BM25/TF-IDF) retrieval with Reciprocal Rank Fusion
- **Graph relationships** for multi-hop traversal and semantic navigation
- **Multi-transport APIs** (REST, gRPC, VectorizerRPC binary, GraphQL, MCP)
- **Native AI integration** via 31 MCP tools for Cursor, Claude Desktop, and other AI IDEs

## Maturity & Positioning

Vectorizer is **production-ready** and shipping in HiveLLM v2.1.0+. It:
- Achieves **4-5x faster search** than Qdrant in benchmarks (0.16-0.23ms vs 0.80-0.87ms)
- Handles 4,400-6,000 QPS with SIMD acceleration (AVX2) and optional Metal GPU (macOS)
- Provides **20-30% on-disk space savings** via `.vecdb` unified format and Product Quantization (64x compression)
- Integrates with Cortex's vector lane for decision context and retrieval workflows
- Supplies embeddings to downstream analysis (nexus graph enrichment, synap prompt compression, expert routing)

## Stack & Architecture

**Language & Runtime:**
- Pure Rust, no external vector libraries (HNSW, quantization implemented in-house)
- Tokio async runtime (multiplexed RPC connections, parallel embedding)
- SIMD dispatch (AVX2, AVX-512, NEON, SVE, WASM) with CPU-feature detection

**Core Crates** (Cargo workspace, `crates/*/` layout):
- `vectorizer-core` — shared error types, quantization, compression, SIMD helpers
- `vectorizer-protocol` — RPC wire types + tonic-generated gRPC stubs
- `vectorizer` — engine (HNSW, embedding models, cache, persistence, search, graph)
- `vectorizer-server` — transport layer (HTTP/REST, RPC, MCP, gRPC, GraphQL)
- `vectorizer-cli` — CLI tools + binary entry points

**SDKs** (first-party, re-export protocol types):
- Rust (v3.3.0, default)
- TypeScript / JavaScript (v3.0.x)
- Python (v3.0.x)
- Go (v3.0.x)
- C# (v3.0.x)

## Key Transports

| Transport | Port | Best for | Overhead |
|-----------|------|----------|----------|
| REST API | 15002 | HTTP clients, browsers, ops tooling | JSON framing + TLS |
| **VectorizerRPC** | 15503 | SDKs, bulk ingest, embedded use | u32 length + MessagePack (default) |
| gRPC | (15002) | Qdrant-compatible polyglot services | HTTP/2 + protobuf |
| MCP | 15002/mcp | AI IDE integration (StreamableHTTP) | JSON-RPC 2.0 over HTTP |
| GraphQL | 15002/graphql | Structured query clients | GraphiQL introspection |

## Repo Layout

```
├── README.md                   # Feature overview & quick start
├── Cargo.toml                  # Workspace definition (5 crates + sdks/rust)
├── CHANGELOG.md                # Release notes (v3.3.0 highlights above)
├── AGENTS.md                   # Rulebook governance (inherited from HiveLLM)
├── crates/
│   ├── vectorizer-core/        # Errors, codec, quantization, SIMD
│   ├── vectorizer-protocol/    # RPC + gRPC wire types
│   ├── vectorizer/             # Engine (db, embedding, cache, search)
│   ├── vectorizer-server/      # HTTP, RPC, MCP, gRPC routers
│   └── vectorizer-cli/         # CLI binaries
├── sdks/rust/                  # Rust SDK (re-exports protocol)
├── docs/
│   ├── users/                  # Installation, tutorials, operations
│   ├── specs/                  # Architecture, performance, migration
│   ├── future/                 # Roadmap (clustering, sharding)
│   └── api/                    # API reference, cluster docs
├── gui/                        # React dashboard (embedded ~26MB binary)
├── config/                     # Example configurations
├── docker-compose.yml          # Multi-profile deployments
└── scripts/                    # Install, benchmarking, CI
```

## Integration with HiveLLM

Vectorizer is consumed by:
- **Cortex** — vector lane for decision context and semantic retrieval
- **CompressionPrompt** — embedding + sparse index for context compression
- **Nexus** — graph enrichment and external-ID correlation
- **Synap** — embedding availability for prompt synthesis

All downstream services use the same `vectorizer-sdk` to connect (auth via `CORTEX_VECTORIZER_API_KEY` or JWT).
