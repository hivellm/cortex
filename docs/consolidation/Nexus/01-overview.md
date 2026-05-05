# Nexus: Overview

## Purpose & Role

Nexus is a **high-performance property graph database** designed for read-heavy workloads with **native vector search** as a first-class feature. It combines Neo4j-compatible Cypher language with approximate nearest-neighbor (HNSW) indexes, enabling hybrid **RAG**, **recommendation**, and **knowledge-graph** applications.

In the HiveLLM ecosystem, Nexus serves as the **graph storage and query backbone** for systems that need to combine structured relationships with semantic embeddings.

## Key Characteristics

- **Language**: Rust (nightly 1.85+)
- **Edition**: 2024
- **License**: Apache-2.0
- **Repository**: github.com/hivellm/nexus
- **Current version**: 2.2.0 (released 2026-05-04)
- **Status**: Production-ready; 2310+ workspace tests passing

## Core Capabilities

1. **Neo4j-compatible Cypher** — 300/300 Neo4j diff-suite tests pass; MATCH, CREATE, MERGE, DELETE, UNWIND, WITH, RETURN, ORDER BY, LIMIT, UNION, pattern comprehensions, list/map comprehensions, EXISTS subqueries.
2. **APOC ecosystem** — ~100 procedures across coll, map, text, date, schema, util, convert, number, agg namespaces.
3. **Native KNN via HNSW** — per-label vector indexes (cosine, L2, dot metrics); bytes-native embeddings on RPC wire.
4. **Binary RPC default** — length-prefixed MessagePack (port 15475); 3–10× lower latency vs HTTP/JSON.
5. **Three transports** — Binary RPC (`nexus://`), HTTP/JSON, RESP3 (debug port).
6. **Full-text search** — Tantivy backend with per-index analyzer catalogue, BM25 ranking, WAL integration.
7. **Constraints** — UNIQUE, NODE KEY, NOT NULL, property-type enforcement.
8. **External node IDs** — caller-supplied stable identifiers (`_id`) with conflict resolution (ERROR, MATCH, REPLACE).
9. **Multi-database** — isolated databases in a single instance.
10. **Sharded cluster (V2)** — hash-based partitioning, per-shard Raft consensus, distributed coordinator.

## Project Layout

```
nexus/
├── crates/                # Rust workspace
│   ├── nexus-core/        # Graph engine (catalog, storage, WAL, indexes, executor)
│   ├── nexus-server/      # Axum HTTP + RPC + RESP3 server
│   ├── nexus-protocol/    # Wire types (REST, MCP, UMICP, RPC)
│   ├── nexus-cli/         # RPC-default CLI binary
│   └── nexus-bench/       # Neo4j vs Nexus benchmarks
├── sdks/                  # 6 first-party SDKs (Rust, Python, TypeScript, Go, C#, PHP)
├── tests/                 # Integration + Neo4j compatibility
├── docs/                  # Guides, specs, compatibility, performance
└── deploy/                # Helm, Docker Compose, Kubernetes
```

## Six First-Party SDKs

All track the same 2.2.0 cadence. Default to `nexus://` (binary RPC), fall back to `http://`.

| Language | Install | Status |
|----------|---------|--------|
| Rust | `nexus-sdk = "2.2.0"` | ✅ shipped |
| Python | `pip install hivehub-nexus-sdk` | ✅ shipped |
| TypeScript | `npm install @hivehub/nexus-sdk` | ✅ shipped |
| Go | `go get github.com/hivellm/nexus-go` | ✅ shipped |
| C# | `dotnet add package Nexus.SDK` | ✅ shipped |
| PHP | `composer require hivellm/nexus-php` | ✅ shipped |

## Maturity & Production Readiness

- **Benchmarks**: 56/56 head-to-head wins vs Neo4j 5.15 (median 4.7× faster).
- **Compatibility**: 300/300 Neo4j 2025.09.0 diff-suite tests.
- **SDK validation**: 87/87 live SDK test cases on `hivehub/nexus:2.1.0`.
- **Test coverage**: 2310+ workspace tests; 95%+ coverage expected for new code.
- **Deployment**: Docker (official `hivehub/nexus:2.2.0`), Helm chart, Kubernetes guides.
