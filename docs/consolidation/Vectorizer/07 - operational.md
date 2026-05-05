# Vectorizer — Operational Guide

**Last Updated:** 2026-05-04

## Docker Deployment

### Official Images

| Registry | Image | Notes |
|----------|-------|-------|
| Docker Hub | `hivehub/vectorizer:latest` | Primary, multi-arch (x86_64, arm64) |
| GHCR | `ghcr.io/hivellm/vectorizer:latest` | Same content, alternative registry |
| Version tags | `hivehub/vectorizer:3.3.0` | Specific release, immutable |
| BuildKit cache | `hivehub/vectorizer-cache:buildx` | Optional, speeds local multi-arch builds |

### Quick Start

```bash
docker run -d \
  --name vectorizer \
  -p 15002:15002 -p 15503:15503 \
  -v vectorizer-data:/vectorizer/data \
  -e VECTORIZER_AUTH_ENABLED=true \
  -e VECTORIZER_ADMIN_USERNAME=admin \
  -e VECTORIZER_ADMIN_PASSWORD=your-secure-password \
  -e VECTORIZER_JWT_SECRET=$(openssl rand -hex 64) \
  --restart unless-stopped \
  hivehub/vectorizer:latest
```

### Docker Compose Profiles

**Standard (default):**
```bash
docker compose --profile default up -d
```
Standalone Vectorizer, REST + RPC on 15002/15503.

**Development:**
```bash
docker compose --profile dev up -d
```
Adds hot-reload, verbose logging, insecure auth (loopback bypass).

**High Availability (Raft):**
```bash
docker compose --profile ha up -d
```
3-node Raft cluster with automatic failover.

**HiveHub Cluster:**
```bash
docker compose --profile hub up -d
```
Multi-tenant cluster mode with quotas and tenant isolation.

**Important:** Profiles are mutually exclusive on port 15002. Choose one.

### Build Pipeline

**Docker images built by:**
1. BuildKit multi-arch (native arm64 CI, no QEMU)
2. Buildx registry cache on `hivehub/vectorizer-cache:buildx` (optional `-NoCache` flag)
3. Dedicated `release-docker` Cargo profile (LTO off, codegen-units=16 for speed)
4. SBOM from BuildKit syft attestation (no in-image `cargo sbom`)

**Build time:**
- Cold: 30-45 min (full dependency build)
- Warm (with cache): < 10 min

**Operator runbook:** `docs/development/docker-builds.md`

## Network Topology

### Port Bindings

| Port | Protocol | Service | Notes |
|------|----------|---------|-------|
| **15002** | HTTP 1.1 / 2.0 | REST API, gRPC, MCP, GraphQL, Dashboard | ALPN negotiation |
| **15503** | TCP (custom framing) | VectorizerRPC (binary MessagePack) | Default for SDKs |
| **9090** | HTTP | Prometheus metrics (optional) | Not exposed by default |

### Firewall Rules

**For Cortex (typical):**
```
Cortex API → Vectorizer:15002 (HTTP, REST + gRPC)
Cortex API → Vectorizer:15503 (TCP, RPC optional)
```

**For cluster (Raft):**
```
Vectorizer Leader ↔ Vectorizer Replica:15002 (HTTP for heartbeat)
Vectorizer Replica ↔ Vectorizer Leader:15503 (RPC for WAL replication)
```

**For dashboard (local only):**
```
Admin Browser → Vectorizer:15002/dashboard/ (HTTP, localhost only by default)
```

## Configuration

### Layered Config Loader

**Files (in merge order):**
1. `config/config.yml` — base configuration (your deployment)
2. `config/modes/{$VECTORIZER_MODE}.yml` — overlay (production/dev/ha)
3. Environment variables (override everything)

**Example:**
```bash
VECTORIZER_MODE=production ./vectorizer
# Equivalent to:
# - Load config/config.yml (base)
# - Merge config/modes/production.yml (override)
# - Apply env var overrides
```

### Essential Environment Variables

| Variable | Purpose | Example |
|----------|---------|---------|
| `VECTORIZER_MODE` | Config mode (production / dev / ha) | `production` |
| `VECTORIZER_DATA_DIR` | Data directory | `/vectorizer/data` |
| `VECTORIZER_LOG_LEVEL` | Log level (trace / debug / info / warn / error) | `info` |
| `VECTORIZER_AUTH_ENABLED` | Enable JWT + API keys | `true` |
| `VECTORIZER_ADMIN_USERNAME` | Root username (first-run only) | `admin` |
| `VECTORIZER_ADMIN_PASSWORD` | Root password (first-run only, ≥32 chars) | *(generated)* |
| `VECTORIZER_JWT_SECRET` | JWT signing secret (≥32 chars) | `$(openssl rand -hex 64)` |
| `VECTORIZER_RPC_ENABLED` | Enable VectorizerRPC on port 15503 | `true` |
| `VECTORIZER_REST_ENABLED` | Enable REST on port 15002 | `true` |

### Config Structure (YAML)

**Base config example:**
```yaml
server:
  http:
    port: 15002
    timeout_secs: 30
  rpc:
    port: 15503
    enabled: true

auth:
  enabled: true
  jwt_secret: "..." # ≥32 chars
  jwt_expiry_secs: 3600
  cookies:
    insecure_dev: false  # Only for loopback, dev-mode
  dev_mode_skip_loopback: false  # !WARNING: loopback only

memory:
  max_cache_memory_bytes: 4294967296  # 4GB
  enable_mmap: true

snapshots:
  enabled: true
  interval_secs: 3600
  compression: zstd
  zstd_level: 12

replication:
  raft:
    enabled: false
    node_id: "node-1"
  master_replica:
    enabled: false
    sync_interval_ms: 100

cluster:
  enabled: false
  sharding_enabled: false

logging:
  level: info
  format: json  # or "text"
  outputs:
    - stdout
    - file:///vectorizer/logs/vectorizer.log
```

**Mode overlays:**
- `config/modes/production.yml` — higher thread limits, zstd compression
- `config/modes/dev.yml` — verbose logging, file watcher, loopback auth bypass
- `config/modes/ha.yml` — Raft consensus, 3+ nodes

## Monitoring & Observability

### Health Check

```bash
curl http://localhost:15002/health
# Response: {"status":"healthy","version":"3.3.0"}
```

No auth required. Boot probe uses this to validate connection.

### Metrics (Prometheus)

**Endpoint:** `GET http://localhost:9090/metrics` (if enabled)

**Key metrics:**
- `vectorizer_search_latency_ms` — histogram (p50, p95, p99)
- `vectorizer_insert_latency_ms` — histogram
- `vectorizer_collections_total` — gauge
- `vectorizer_vectors_total` — gauge
- `vectorizer_cache_hit_ratio` — gauge
- `vectorizer_rpc_connections` — gauge
- `vectorizer_auth_jwt_validations` — counter
- `vectorizer_api_key_usage` — counter (per key)

**Scrape config (Prometheus):**
```yaml
scrape_configs:
  - job_name: 'vectorizer'
    static_configs:
      - targets: ['localhost:9090']
```

### Logging

**Log files:**
- Linux: `~/.local/share/vectorizer/logs/`
- macOS: `~/Library/Application Support/vectorizer/logs/`
- Windows: `%APPDATA%\vectorizer\logs\`
- Override: `VECTORIZER_LOGS_DIR` env var

**Formats:** JSON (prod) or text (dev)

**Key log messages:**
```
[INFO] vectorizer booting...
[INFO] auth: JWT secret ≥32 chars ✓
[INFO] collections loaded: 42 (workspace + dynamic)
[WARN] auth: anonymous mode (no credentials configured)
[ERROR] probe_authenticated 401: credentials wrong?
[INFO] replication: Raft node-1 elected leader
```

### Dashboard

**URL:** `http://localhost:15002/dashboard/`

**Access:** Localhost only (default) or via JWT after login

**Features:**
- Collections CRUD
- Vector search / graph traversal
- API key management + usage sparklines
- Audit log viewer
- Cluster status (if Raft enabled)

**First-run credentials:** Emitted to `{data_dir}/.root_credentials` (0o600), never stdout.

## Authentication & Credentials

### First-Run Setup

1. **Boot requires credentials:**
   ```bash
   VECTORIZER_ADMIN_USERNAME=admin \
   VECTORIZER_ADMIN_PASSWORD=your-secure-password \
   VECTORIZER_JWT_SECRET=$(openssl rand -hex 64) \
   ./vectorizer
   ```

2. **Root credentials saved:**
   ```
   {data_dir}/.root_credentials (mode 0o600)
   Content: {"username":"admin","password":"..."}
   ```
   Read once, then delete (never re-logged).

3. **Create service API keys:**
   ```bash
   curl -X POST http://localhost:15002/auth/login \
     -d '{"username":"admin","password":"..."}'
   # Returns JWT
   
   curl -X POST http://localhost:15002/auth/keys \
     -H "Authorization: Bearer $JWT" \
     -d '{"name":"cortex-api","permissions":["read","write"],"expires_in_days":90}'
   # Returns API key
   ```

### Cortex Integration

**Env vars (resolved in strict order):**
1. `CORTEX_VECTORIZER_API_KEY` (or `VECTORIZER_API_KEY`) — bearer
2. `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` — login
3. `CORTEX_EMBEDDER_VECTORIZER_USER` + `_PASSWORD` — alias for (2)
4. *(none)* — anonymous (warns, falls back to MemoryVectorLane)

**JWT refresh strategy:**
- Reactive (first 401 triggers refresh)
- Optional warmup loop (env: `CORTEX_VECTORIZER_JWT_WARMUP_SECS`)

See: `docs/operations/vectorizer-auth.md` (Cortex repo)

## Backup & Disaster Recovery

### Manual Backup

```bash
# Full collection backup
curl -X POST http://localhost:15002/collections/my_docs/backup \
  -o backup.tar.gz
```

### Automated Backup

```yaml
backup:
  enabled: true
  schedule: "0 2 * * *"       # 2 AM daily (cron)
  retention_days: 30           # Keep last 30 days
  compression: true            # gzip
  incremental: true            # Delta after first full
  target: "s3://bucket/path"   # Optional S3 destination
```

### Point-in-Time Recovery

```bash
vectorizer recovery --collection my_docs --until "2025-10-15T09:00:00Z"
```

Replays WAL up to specified timestamp.

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|--------------|-----|
| `probe_authenticated 401` | Wrong credentials | Check username/password against `/auth/login` |
| `WARN vector_lane: no credentials configured` | Cortex: missing auth env vars | Set `CORTEX_VECTORIZER_API_KEY` or user + password |
| Slow search (> 10ms) | Too many vectors, high ef_search | Increase cache size, tune ef_search down |
| OOM on large dataset | Vectors in memory, no MMap | Enable `memory.enable_mmap: true` |
| Raft stuck (no leader) | Network partition or split brain | Check node health, restore from snapshot |
| Corrupted `.vecdb` | Unclean shutdown or disk error | Restore from last snapshot, replay WAL |

## Scaling & HA

### Vertical Scaling (Single Node)

- Increase cache: `memory.max_cache_memory_bytes`
- Tune HNSW: `hnsw_config.ef_search` (higher = more accurate, slower)
- Enable PQ: Quantization reduces memory 64x

### Horizontal Scaling (Replication)

**Raft cluster (3+ nodes, automatic failover):**
```yaml
replication:
  raft:
    enabled: true
    node_id: "node-1"
    cluster: ["node-1:15002", "node-2:15002", "node-3:15002"]
```

**Master-Replica (write to leader, read from replicas):**
```yaml
replication:
  master_replica:
    enabled: true
    role: "master"  # or "replica"
    master_url: "http://leader:15002"
    sync_interval_ms: 100
```

### Sharding (Distributed)

```yaml
cluster:
  enabled: true
  sharding_enabled: true
  shard_count: 4  # 4 shards across nodes
  rebalance_interval_secs: 300
```

Automatic routing: client → correct shard (hash-based).

## Release & Upgrade

### Version Pinning

Docker Compose:
```yaml
services:
  vectorizer:
    image: hivehub/vectorizer:3.3.0  # Pin to specific version
```

Rust SDK:
```toml
[dependencies]
vectorizer-sdk = "3.3"  # Compatible versions (3.3.x)
```

### Breaking Changes

| Version | Breaking Change | Migration |
|---------|-----------------|-----------|
| v3.0 | RPC became default | Set `rpc.enabled: false` in config if REST-only |
| v3.1 | API key usage metrics (additive) | Old keys deserialized with `usage_count: 0` |
| v3.2 | Cluster failover endpoints | No action required (additive) |
| v3.3 | CSRF middleware on mutating requests | Dashboard updated automatically; SDKs unchanged |

### Upgrade Path

1. Test on staging (same config as prod)
2. Upgrade image/binary
3. Restart container
4. Verify health check (`GET /health`)
5. Run smoke test (one search per collection)

Backward-compatible: old clients work with new servers.
