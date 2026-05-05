# Synap — Cortex Relevance

## Data Ingest Points

### 1. Event Streams for Task Distribution
- **What**: Cortex workers publish task results to Synap streams
- **How**: `cortex-workers` → Synap stream (room-based)
- **Benefit**: Distributed task coordination, replay, retention policies

### 2. Queue for Worker Pool
- **What**: Background job distribution (embedding jobs, classification)
- **How**: Cortex ingest pipeline publishes to Synap queue
- **Benefit**: ACK-based delivery guarantee, priority routing, DLQ for failures

### 3. Pub/Sub for Real-Time Notifications
- **What**: Alert broadcasts (classification complete, pipeline stage transitions)
- **How**: Cortex workers publish to `cortex/classifier/*` topic
- **Benefit**: Zero-coupling between components, wildcard subscriptions

### 4. KV Cache for Computed Results
- **What**: Cache embeddings, classification results, intermediate state
- **How**: Store in Synap KV with TTL (expire after 24h)
- **Benefit**: Fast retrieval, TTL auto-cleanup, SIMD-accelerated BITCOUNT

### 5. Metrics & Monitoring
- **What**: Capture pipeline latency, throughput, errors
- **How**: Log to Synap stream (raw events) + query Prometheus metrics
- **Benefit**: Append-only audit trail, Prometheus integration

## Data Schema for Cortex

### Stream Events (cortex/results/{job_id})
```json
{
  "id": 12345,
  "room": "cortex/results/job-uuid",
  "eventType": "classification_complete",
  "data": {
    "job_id": "job-uuid",
    "status": "success",
    "result": { ... },
    "latency_ms": 1234
  },
  "timestamp": "2026-05-04T10:30:00Z"
}
```

### Queue Messages (cortex-ingest)
```json
{
  "queue": "cortex-ingest",
  "payload": { ... },
  "priority": 7,
  "maxRetries": 3
}
```

## Integration Points

1. **cortex-workers** — Publishes results to streams/queues
2. **cortex-api** — Reads KV cache for fast lookups
3. **cortex-core** — Uses queue for task dispatch
4. **cortex-cli** — Subscribes to streams for live monitoring

## Performance Targets

- **Stream Publish**: < 1ms (latency target)
- **Queue Consume**: < 0.5ms
- **KV GET**: 87ns (already 20,000x target)
- **Replication Lag**: < 10ms (typical)

## No Coupling Guarantee

- Synap is **standalone**, replaceable with Redis/RabbitMQ if needed
- SDKs are official (Rust, Python), not custom bindings
- Protocol is stable (HTTP + synap:// + resp3://)
