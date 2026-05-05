# Synap — Key Design Decisions

## Language: Rust Edition 2024

**Decision**: Use Rust (nightly 1.85+) as sole implementation language.

**Rationale**:
- Memory safety without garbage collection (critical for microsecond latency)
- Native performance matching C/C++
- Fearless concurrency (ownership prevents data races)
- Mature async ecosystem (Tokio)
- Proven in production (Cloudflare, Discord, AWS services)

**Alternative Rejected**: Go (GC pauses unacceptable for sub-microsecond targets)

## Async Runtime: Tokio

**Decision**: Tokio (not async-std, smol, custom).

**Rationale**:
- Work-stealing scheduler for 64-way sharding
- Battle-tested in production
- Largest async ecosystem (Axum, Hyper, Tower)

## Web Framework: Axum

**Decision**: Axum (not actix-web, warp, hyper).

**Rationale**:
- Built on Tokio/Tower/Hyper stack
- Compile-time request/response validation (extractors)
- WebSocket support
- Type-safe routing

## Data Structure: Radix Trie

**Decision**: `radix_trie` crate for KV store (not HashMap, BTreeMap).

**Rationale**:
- O(k) lookup by key length (vs O(1) hash but with better cache locality)
- Memory-efficient (54% reduction vs HashMap at scale)
- Prefix-based KEYS/SCAN operations natural
- No hashing overhead

## Multi-Protocol Support

**Decision**: Support HTTP, synap:// (MessagePack), resp3:// (Redis) simultaneously.

**Rationale**:
- HTTP for webhooks, ad-hoc `curl`
- `synap://` for production (persistent TCP, binary, type preservation)
- `resp3://` for Redis-compatible tooling migration path

## Queue: FIFO with Acknowledgment

**Decision**: RabbitMQ-style ACK model (not Kafka consumer groups for queues).

**Rationale**:
- Explicit acknowledgment prevents message loss
- Retry logic on NACK
- Dead Letter Queue for failed messages
- Zero-duplicate guarantee (tested with concurrency)

## Master-Slave Replication

**Decision**: Master-write, replicas read-only (not quorum-based Raft).

**Rationale**:
- Simpler operation and understanding
- Eventual consistency acceptable for cache layer
- Lower latency (no quorum round-trips)
- Manual failover is acceptable for operational tasks
- Tested with 3+ replicas

## Persistence: WAL + Snapshots

**Decision**: Append-only WAL + periodic snapshots (not RDB-only).

**Rationale**:
- WAL catches all mutations (KV, queue, stream)
- Snapshots provide fast recovery (1–10s from disk)
- Combined model is PACELC-compliant (PC/EL)
- Lower space overhead than pure snapshot

## Authentication: User + API Keys

**Decision**: SHA512 hashing, API key expiration, IP filtering.

**Rationale**:
- Production-grade security (not plaintext)
- API key rotation and per-IP restriction
- Audit logging for compliance
- Fine-grained permissions (resource-based ACL)
