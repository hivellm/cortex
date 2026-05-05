# Lexum Overview

## Purpose & Role

**Lexum** is a high-performance, distributed full-text search engine written in Rust, designed as a modern alternative to Elasticsearch with cloud-native architecture.

**Primary Use Case**: Enterprise-grade full-text indexing and search operations with support for complex queries, aggregations, and multi-protocol access.

## Status & Maturity

- **Version**: 0.1.0-alpha
- **Overall Progress**: 38% complete (Phase 1: Foundation 100% done)
- **Production Readiness**: Foundation complete (core search, REST API, CLI, query language)
- **Test Coverage**: 278+ tests passing, >95% coverage for implemented features
- **Code Size**: ~93,000 lines of Rust across 129 files

## Stack & Dependencies

- **Language**: Rust 2024 Edition (1.85+)
- **Search Engine**: Tantivy 0.25 (Lucene-inspired full-text indexing)
- **Runtime**: Tokio 1.48 (async executor)
- **Web Framework**: Axum 0.8 + Tower middleware
- **Database**: RocksDB (metadata storage)
- **Serialization**: serde, serde_json, serde_yaml, bincode
- **Query Language**: Custom LQL (SQL-inspired syntax)
- **API Documentation**: utoipa 5.4 + OpenAPI 3.0

## Key Features (Implemented)

- **Search**: Full-text indexing with BM25 scoring, fuzzy matching, phrase queries
- **Query Language**: LQL with 9 query types (SELECT, JOIN, etc.)
- **REST API**: 39 endpoints, 100% working, OpenAPI/Swagger UI
- **CLI**: 8 command groups (index, document, snapshot, repl, etc.)
- **Snapshots**: Complete backup/restore with repository management
- **Templates**: Automatic index configuration with pattern matching
- **Security**: API key auth, rate limiting, CORS
- **Monitoring**: Cluster health, statistics, node monitoring
- **Performance**: Query cache, concurrent search, compression

## Architecture Highlights

- **Layered Design**: Client → Protocol → Gateway → Coordination → Query → Index → Storage
- **Sharding**: Hash-based document distribution across nodes
- **Replication**: Primary-replica model with configurable consistency levels
- **Protocols**: HTTP/2 (StreamableHTTP), MCP, UMICP support planned
- **Scalability**: Horizontal (more nodes) + vertical (more resources per node)

## Design Philosophy

1. **Type-Safe**: Rust's guarantees eliminate entire classes of bugs
2. **Performance**: Zero-cost abstractions, minimal allocations
3. **Cloud-Native**: Stateless nodes, distributed consensus (Raft)
4. **SQL Familiarity**: LQL syntax familiar to developers from SQL/Elasticsearch backgrounds
