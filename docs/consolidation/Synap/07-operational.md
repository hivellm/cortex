# Synap — Operational

## Docker Deployment

### Image
- **Registry**: `hivehub/synap` (Docker Hub)
- **Latest**: `hivehub/synap:latest`
- **Versioned**: `hivehub/synap:0.12.0`
- **Architectures**: `linux/amd64`, `linux/arm64`

### Ports
- **HTTP REST**: 15500
- **SynapRPC (TCP/MessagePack)**: 15501
- **RESP3 (Redis)**: 6379
- **Replication (Master)**: 15501 (same as SynapRPC)

### Environment Variables (Docker)
```bash
SYNAP_AUTH_ENABLED=true
SYNAP_AUTH_REQUIRE_AUTH=true
SYNAP_AUTH_ROOT_USERNAME=admin
SYNAP_AUTH_ROOT_PASSWORD=SecurePassword123!
SYNAP_AUTH_ROOT_ENABLED=true
```

### Quick Start
```bash
docker run -d \
  --name synap \
  -p 15500:15500 \
  -p 15501:15501 \
  -v synap-data:/data \
  hivehub/synap:latest

curl http://localhost:15500/health
```

## Configuration

**File**: `config.yml` (YAML)

Key sections:
```yaml
server:
  host: "0.0.0.0"
  port: 15500
  
replication:
  enabled: true
  role: "master" | "replica"
  master_address: "master:15501"  # for replicas
  
persistence:
  enabled: true
  wal:
    path: "./data/wal/synap.wal"
  snapshot:
    directory: "./data/snapshots"

auth:
  enabled: true
  require_auth: true
  root:
    username: "admin"
    password: "..."
```

## Monitoring

### Health Check
```bash
curl http://localhost:15500/health
```

### INFO Command
```bash
curl http://localhost:15500/admin/info
# Returns: server, memory, stats, replication, keyspace
```

### SLOWLOG
```bash
curl http://localhost:15500/admin/slowlog?threshold_ms=10
```

### Prometheus Metrics
- 17 metric types (ops/sec, latency, memory, connections)
- Endpoint: `/metrics` (Prometheus format)

### Replication Status
```bash
curl http://localhost:15500/health/replication
# Returns: lag, offset, connected replicas
```

## Scaling

### Vertical Scaling
- Increase server resources (CPU, RAM)
- Sharding is internal (64-way automatic)

### Horizontal Scaling (Read)
- Deploy replicas for read-only load balancing
- Typical lag: 5–10ms

### Write Scaling
- Single master (writes not distributed)
- Queue partitioning or sharding at application layer

## Persistence Modes

### fsync Modes
- **Always**: Every operation synced (slowest, safest)
- **Periodic**: Sync every 1 second (default)
- **Never**: No fsync (fastest, risky)

### Recovery
- Automatic on startup
- Snapshot + WAL replay
- Typical recovery: 1–10 seconds
