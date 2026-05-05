# Synap — Architecture

## High-Level Layers

```
┌─────────────────────────────────────────────┐
│ Client Layer (SDKs: Rust, TS, Python, etc.) │
├─────────────────────────────────────────────┤
│ Protocol Layer (HTTP, synap://, resp3://)   │
├─────────────────────────────────────────────┤
│ Command Router & Handlers                   │
├─────────────────────────────────────────────┤
│ Core Subsystems (KV, Queue, Stream, PubSub)│
├─────────────────────────────────────────────┤
│ Persistence (WAL, Snapshots)                │
├─────────────────────────────────────────────┤
│ Replication Layer (Master→Replicas)         │
└─────────────────────────────────────────────┘
```

## Core Subsystems

### Key-Value Store
- **Structure**: radix_trie (O(k) lookup by key length)
- **Features**: GET, SET, DEL, INCR, DECR, MGET, MSET, EXPIRE, TTL, SCAN
- **Data Types**: Hashes, Lists, Sets, Sorted Sets, Bitmaps, HyperLogLog, Geospatial
- **Sharding**: 64-way internal sharding for concurrency

### Queue System
- **Pattern**: FIFO with acknowledgment (RabbitMQ-style)
- **Features**: Priority (0–9), ACK/NACK, Dead Letter Queue, retry logic
- **Durability**: Persistent via WAL + snapshots

### Event Streams
- **Pattern**: Ring buffer per "room" (Kafka-style partitioned topics)
- **Features**: Append-only logs, Consumer Groups, retention policies
- **Scalability**: Multiple partitions per topic

### Pub/Sub Router
- **Pattern**: Topic-based with wildcard subscriptions
- **Features**: Wildcard matching (`*`, `#`), hierarchical topics
- **Performance**: ~850K msgs/s, 1.2µs latency

## Transport Protocols

| Scheme | Port | Wire Format | Use Case |
|--------|------|-------------|----------|
| `synap://` | 15501 | MessagePack/TCP | Recommended (persistent, binary, type-safe) |
| `resp3://` | 6379 | Redis text | Redis tooling, existing clients |
| `http://` | 15500 | JSON/HTTP | Ad-hoc `curl`, webhooks |

## Deployment Model

- **Single Server**: Standalone with persistence
- **Replication**: 1 master + N read replicas (TCP binary protocol)
- **Monitoring**: INFO, SLOWLOG, MEMORY USAGE, CLIENT LIST, Prometheus metrics
