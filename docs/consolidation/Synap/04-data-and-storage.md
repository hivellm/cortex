# Synap — Data & Storage

## State Ownership

### In-Memory State
- **KV Store**: radix_trie of all keys, values, TTL metadata
- **Queues**: FIFO message buffers, ACK tracking, retry counters
- **Streams**: Ring buffers per room, offset tracking per consumer group
- **Pub/Sub**: Subscription tree (topic→subscribers)
- **Authentication**: User/API key database (SHA512 hashing)

### Persistent State
Persisted via append-only WAL + periodic snapshots.

## Persistence Architecture

### WAL (Write-Ahead Log)
- **Format**: Bincode-encoded operations
- **Batching**: Groups up to 10K ops, flushed within 100µs
- **Fsync Modes**: Always, Periodic (1s), Never
- **Storage**: Single file per node (`./data/wal/synap.wal`)

### Snapshots
- **Format**: Complete KV store + queue state + stream metadata
- **Creation**: On-demand (`POST /snapshot`) or periodic
- **Recovery**: Snapshots + WAL replay on startup
- **CRC32**: Verification during replication transfer

## Replication Log

- **Type**: Circular buffer (1M operations, like Redis)
- **Contents**: All KV writes, queue messages, stream events
- **Purpose**: Enable partial sync (replication offset tracking)
- **Master Exports**: Full/partial snapshots to replicas

## Data Schemas

### KV Entry
```
Key: String
Value: Vec<u8>
TTL: Option<Instant>
Metadata: HashMap<String, String>
```

### Queue Message
```
MessageId: UUID
Payload: Vec<u8>
Priority: 0–9
AckDeadline: Instant
RetryCount: u32
State: Pending|Acked|Nacked
```

### Stream Event
```
Id: u64 (offset)
Room: String
EventType: String
Data: Vec<u8>
Timestamp: Instant
```

## Configuration

Storage configured via `config.yml`:
```yaml
persistence:
  enabled: true
  wal:
    enabled: true
    path: "./data/wal/synap.wal"
  snapshot:
    enabled: true
    directory: "./data/snapshots"
```

## PACELC Model

- **PC (Partition + Consistency)**: Master unavailable → replicas are read-only
- **EL (Eventual Consistency + Latency)**: Normal operation → async replication lag
