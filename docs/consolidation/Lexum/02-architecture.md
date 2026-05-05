# Lexum Architecture

## System Layers

```
Client Layer (GUI / REST / MCP / UMICP)
    ↓
Protocol Layer (HTTP/2, MCP, UMICP, WebSocket)
    ↓
API Gateway (routing, auth, rate limiting, circuit breaker)
    ↓
Coordination Layer (cluster mgmt, shard routing, replication)
    ↓
Query Layer (LQL parser, planner, aggregation, merger)
    ↓
Index Layer (Tantivy-powered indexing & search)
    ↓
Storage Layer (filesystem, RocksDB metadata, S3/blob backup)
```

## Core Components

### API Gateway
- Request routing and load balancing
- TLS termination
- Authentication (API key, OAuth 2.0, mTLS)
- Authorization (RBAC, document-level, field-level)
- Rate limiting and connection pooling
- Built with Axum + Tokio

### Coordination Layer
- **Cluster Manager**: Node discovery, health monitoring, leader election (Raft)
- **Shard Router**: Hash-based shard assignment, routing tables, rebalancing
- **Replica Manager**: Synchronization, automatic failover, consistency

### Query Layer
- **LQL Parser**: Recursive descent parser with type checking
- **Query Planner**: Filter pushdown, predicate reordering, index selection
- **Aggregation Engine**: Terms, stats, histogram, date histogram, nested, pipeline
- **Result Merger**: Score-based merging, distributed sorting, top-K selection

### Index Layer (Tantivy-powered)
- **Indexing**: Document analysis, inverted index, segment management
- **Search**: BM25 scoring, fuzzy matching, phrase queries, range queries
- **Document Store**: Compressed storage, fast retrieval by ID
- **Field Cache**: Column store for sorting and aggregations

### Storage Layer
- **Filesystem**: Index segments, write-ahead logs, snapshots
- **RocksDB**: Cluster metadata, index metadata, user data, state
- **S3/Blob**: Remote backup, incremental deltas, cross-region replication

## Data Flow Patterns

### Indexing
Client → API Gateway → Auth → Shard Router → Primary/Replicas → Analysis → IndexWrite → WAL+Segments → Ack

### Search
Client → API Gateway → LQL Parser → Query Planner → Shard Router → Parallel Search → Result Merger → Scoring → Aggregations → Client

## Distributed Features

- **Sharding**: `shard_id = hash(document_id) % num_shards`
- **Replication**: Primary + N-1 replicas per shard
- **Consistency**: ONE (primary ack), QUORUM (majority), ALL (all replicas)
- **Failure Handling**: Heartbeat-based detection, replica promotion, automatic rebalancing

## Protocols Supported

- **StreamableHTTP**: HTTP/2 with Server-Sent Events for streaming
- **MCP**: Model Context Protocol for AI/LLM systems
- **UMICP**: Binary protocol with compression, multiplexing, flow control
- **REST API**: Standard HTTP/JSON interface with OpenAPI spec
