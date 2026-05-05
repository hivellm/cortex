# Nexus: Operational

## Docker Image

### Latest Stable
- **Tag**: `hivehub/nexus:2.1.0` (phase 9 + 10 ship train)
- **Also available**: `hivehub/nexus:latest` (floats on every release)
- **Status**: 87/87 live SDK test cases passing; 300/300 Neo4j diff suite

### Quick Start

```bash
# Development (auth disabled on 127.0.0.1)
docker run -p 15474:15474 -p 15475:15475 \
  hivehub/nexus:latest

# Production (auth required on 0.0.0.0)
docker run -p 15474:15474 -p 15475:15475 \
  -e NEXUS_ADDR=0.0.0.0:15474 \
  -e NEXUS_AUTH_ENABLED=true \
  -v nexus-data:/data \
  hivehub/nexus:2.1.0
```

### Volume Mounts
- `/data` — storage root (catalog, stores, WAL, indexes)

### Environment Variables

**Server**:
- `NEXUS_ADDR` — HTTP bind (default: 127.0.0.1:15474)
- `NEXUS_RPC_ADDR` — RPC bind (default: 127.0.0.1:15475)
- `NEXUS_RESP3_ENABLED` — enable RESP3 debug port (default: false)
- `NEXUS_DATA_DIR` — storage path (default: ./data)
- `NEXUS_AUTH_ENABLED` — require auth (auto true for 0.0.0.0)
- `NEXUS_REPLICATION_ROLE` — standalone/master/replica
- `NEXUS_SIMD_DISABLE` — emergency scalar fallback (default: 0)
- `RUST_LOG` — log level (error/warn/info/debug/trace; default: info)

**Client**:
- `NEXUS_URL` — CLI endpoint (nexus://, http://, etc.)
- `NEXUS_TRANSPORT` — force transport (nexus/http/auto)
- `NEXUS_SDK_TRANSPORT` — SDK transport override

## Ports

| Port | Protocol | Use | Default |
|------|----------|-----|---------|
| 15474 | HTTP/JSON | REST API, health, Prometheus | 127.0.0.1 |
| 15475 | Binary RPC | CLI + SDK default transport | 127.0.0.1 |
| 15476 | RESP3 | Debug port (redis-cli compatible) | disabled |

## Startup & Healthcheck

### Health Endpoint
```bash
curl http://localhost:15474/health
# Returns: {"status": "healthy", "version": "2.1.0", "database": "default"}
```

### Log Output (default: info level)
```
[2026-05-04T12:34:56Z INFO nexus_server] Starting Nexus server...
[2026-05-04T12:34:56Z INFO nexus_server] HTTP listener: 127.0.0.1:15474
[2026-05-04T12:34:56Z INFO nexus_server] RPC listener: 127.0.0.1:15475
[2026-05-04T12:34:56Z INFO nexus_core::catalog] Opened catalog: /data/catalog.mdb
[2026-05-04T12:34:56Z INFO nexus_server] Ready
```

## Command-Line Interface

### Install (Pre-Built Binary)

**Linux / macOS**:
```bash
curl -fsSL https://raw.githubusercontent.com/hivellm/nexus/main/scripts/install/install.sh | bash
```

**Windows (PowerShell)**:
```powershell
irm https://raw.githubusercontent.com/hivellm/nexus/main/scripts/install/install.ps1 | iex
```

Puts `nexus-server` and `nexus` (CLI) on PATH.

### CLI Subcommands

```bash
# Query
nexus query "MATCH (n) RETURN count(n)"

# Database management
nexus db list
nexus db create mydb
nexus db switch mydb
nexus db info
nexus db drop mydb

# User + auth
nexus user create alice --password secret
nexus user delete alice
nexus user list

nexus key create myapp
nexus key delete myapp

# Schema
nexus schema show
nexus index list
nexus constraint list

# Data export/import
nexus data export queries.cypher
nexus data import queries.cypher

# Cluster (if v2 sharding enabled)
nexus cluster status
nexus cluster add-node node-b 10.0.0.2:15480
```

### Transport & Authentication

```bash
# Explicit transport
nexus --transport http query "RETURN 1"

# API key auth
nexus --api-key nexus_sk_abc123... query "MATCH (n) RETURN count(n)"

# Username/password
nexus --username alice --password secret query "..."

# Environment variables
NEXUS_URL=http://localhost:15474 nexus query "RETURN 1"
NEXUS_API_KEY=nexus_sk_... nexus query "RETURN 1"
```

## Monitoring

### Prometheus Metrics

Endpoint: `GET http://localhost:15474/prometheus`

**Key metrics**:
- `nexus_query_count` — labeled by statement type (MATCH, CREATE, etc.)
- `nexus_query_duration_seconds` — histogram (p50, p95, p99)
- `nexus_cache_hits_total` / `nexus_cache_misses_total` — L1/L2/L3
- `nexus_cache_hit_ratio` — percentage
- `nexus_rpc_connections` — active connections
- `nexus_index_lookup_duration_seconds` — per-index-type latency
- `nexus_audit_log_failures_total` — failed audit log writes (fail-open)

### Replication Status

```bash
curl http://localhost:15474/replication/status
# Returns: {"role": "master", "lag_bytes": 0, "replicas": [...]}

curl http://localhost:15474/replication/lag
# Real-time lag metrics
```

### Cluster Status (V2)

```bash
curl http://localhost:15474/cluster/status
# Returns: {"layout": {...}, "shards": [...], "generation": 42}
```

## Deployment Topologies

### Single Node (Local Dev)

```bash
docker run -p 15474:15474 -p 15475:15475 \
  -e NEXUS_AUTH_ENABLED=false \
  hivehub/nexus:latest
```

### Master-Replica (High Availability)

**Master**:
```bash
docker run -p 15474:15474 -p 15475:15475 \
  -e NEXUS_REPLICATION_ROLE=master \
  -e NEXUS_REPLICATION_BIND_ADDR=0.0.0.0:15475 \
  -v master-data:/data \
  hivehub/nexus:2.1.0
```

**Replica**:
```bash
docker run -p 15474:15474 -p 15475:15475 \
  -e NEXUS_REPLICATION_ROLE=replica \
  -e NEXUS_REPLICATION_MASTER_ADDR=master:15475 \
  -v replica-data:/data \
  hivehub/nexus:2.1.0
```

Writes go to master; reads can use replicas. Replicas stream WAL from master.

### Kubernetes (Helm)

```bash
helm repo add hivellm https://charts.hivellm.org
helm install my-nexus hivellm/nexus \
  --set appVersion=2.1.0 \
  --set replication.enabled=true
```

Chart: `deploy/helm/nexus/` in the repo.

## Backup & Disaster Recovery

### Data Directory Backup
```bash
tar -czf nexus-backup-$(date +%Y%m%d).tar.gz -C /data .
```

### WAL-Based Recovery
- WAL is append-only; crash recovery automatic
- Replicas stream WAL for incremental backup
- Checkpoints (per-epoch snapshots) in `data/checkpoints/`

### Restore from Backup
```bash
rm -rf /data
tar -xzf nexus-backup-20260504.tar.gz -C /data
# Restart Nexus; WAL replay restores consistency
```

## Configuration (Advanced)

### config.toml Example

```toml
[database]
path = "/data"
cache_size_mb = 512

[http]
bind = "127.0.0.1:15474"

[rpc]
bind = "127.0.0.1:15475"

[auth]
enabled = true

[replication]
role = "standalone"  # or "master", "replica"
master_addr = "master:15475"  # for replicas

[cluster.sharding]
mode = "disabled"  # or "bootstrap", "join"
node_id = "node-a"
listen_addr = "0.0.0.0:15480"
num_shards = 3
replica_factor = 3

[indexes.knn]
default_m = 16
default_ef = 200

[limits]
rate_per_key_per_minute = 1000
rate_per_key_per_hour = 10000
```

## Performance Tuning

### Cache Tuning
- **Page cache size**: Larger for high-throughput workloads
- **Default**: Auto-calculated from available RAM
- **Recommendation**: 50–70% of available memory

### Index Tuning
- **HNSW parameters**: M (degree, default 16), ef (search, default 200)
- **Set per index**: Via index creation parameters
- **Trade-off**: Larger M/ef = better recall, slower construction

### Connection Pool
- **RPC multiplexing**: Single connection supports pipelined queries
- **CLI**: Default 1 connection; reused for sequential commands
- **SDKs**: Pool size configurable (language-specific)

## Troubleshooting

### Port Conflicts
```bash
# Check if ports in use
lsof -i :15474 | grep LISTEN
lsof -i :15475 | grep LISTEN

# Use custom ports
docker run -p 18474:15474 -p 18475:15475 \
  -e NEXUS_ADDR=127.0.0.1:15474 \
  -e NEXUS_RPC_ADDR=127.0.0.1:15475 \
  hivehub/nexus:latest
```

### Auth Issues
```bash
# Disable auth for debugging (local dev only)
docker run -e NEXUS_AUTH_ENABLED=false ...

# Create admin user
nexus --username root --password root user create alice --password secret
nexus --username alice --password secret key create cortex
```

### Out of Memory
```bash
# Increase cache size
docker run -e NEXUS_CACHE_SIZE_MB=2048 ...
# Or reduce: docker run -m 4g ... (container memory limit)
```

### Slow Queries
```bash
# Enable debug logging
docker run -e RUST_LOG=debug ...

# Profile a query
nexus --username root --password root \
  query "PROFILE MATCH (n:File) RETURN count(n)"
```
