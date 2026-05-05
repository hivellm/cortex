# Lexum Operational Guide

## Running Lexum

### Single-Node Server (Development)

```bash
# Build from source
cargo build --release
./target/release/lexum-server

# Or use binary directly
lexum-server --config config.yml
```

**Default Configuration**:
- Host: 127.0.0.1:9200
- Data Directory: ./data
- Logging: info level

### Environment Variables

```bash
# Network
LEXUM_NETWORK_HOST=0.0.0.0          # Bind address
LEXUM_NETWORK_HTTP_PORT=9200        # HTTP port
LEXUM_NETWORK_TRANSPORT_PORT=9300   # Inter-node communication

# Storage
LEXUM_DATA_DIR=/data                # Index data location
LEXUM_SNAPSHOT_DIR=/snapshots       # Snapshot storage

# Configuration
LEXUM_CONFIG_FILE=/app/config.yml   # Config file path

# Logging
RUST_LOG=info                       # Log level (trace, debug, info, warn, error)
RUST_BACKTRACE=1                    # Stack traces on panic

# Performance
LEXUM_WORKER_THREADS=4              # Tokio worker threads
LEXUM_QUERY_CACHE_SIZE=1000         # Query cache entries
```

## Docker Deployment

### Single-Node Container

```bash
# Build image
docker build -t lexum:0.1.0-alpha .

# Run container
docker run -d \
  --name lexum-server \
  -p 9200:9200 \
  -v lexum-data:/data \
  -v lexum-snapshots:/snapshots \
  -e RUST_LOG=info \
  lexum:0.1.0-alpha

# Check health
curl http://localhost:9200/_cluster/health

# View logs
docker logs -f lexum-server

# Stop container
docker stop lexum-server
docker rm lexum-server
```

### Docker Compose (Single-Node)

```bash
# Start
docker-compose up -d

# Check status
docker-compose ps
docker-compose logs -f lexum

# Stop
docker-compose down
docker-compose down -v  # Remove volumes too
```

### Docker Compose (Multi-Node Cluster)

```bash
# Start 3-node cluster
docker-compose -f docker-compose.cluster.yml up -d

# View logs from all nodes
docker-compose -f docker-compose.cluster.yml logs -f

# Stop cluster
docker-compose -f docker-compose.cluster.yml down
```

## Kubernetes Deployment

### Helm Chart

```bash
# Install
helm install lexum ./helm/lexum \
  --namespace lexum \
  --create-namespace

# Verify
kubectl get pods -n lexum
kubectl get svc -n lexum

# Check cluster health
kubectl exec -it lexum-0 -n lexum -- \
  curl localhost:9200/_cluster/health

# Uninstall
helm uninstall lexum -n lexum
```

### Helm Values (Customization)

```yaml
# values.yaml
replicaCount: 3
image:
  repository: lexum
  tag: 0.1.0-alpha
resources:
  requests:
    cpu: 500m
    memory: 1Gi
  limits:
    cpu: 2000m
    memory: 4Gi
persistence:
  enabled: true
  size: 10Gi
```

## Configuration File (config.yml)

```yaml
# Network configuration
network:
  host: 0.0.0.0
  http_port: 9200
  transport_port: 9300
  max_connections: 1000

# Storage paths
storage:
  data_dir: /data
  snapshot_dir: /snapshots

# Cluster settings
cluster:
  name: lexum-prod
  node_name: lexum-node-1

# Search engine
search:
  index_cache_size: 1000000  # Query results cache
  field_cache_size: 100000000  # Aggregation cache
  merge_factor: 20  # Segments merge policy

# Telemetry
telemetry:
  metrics:
    enabled: true
    endpoint: /metrics
    interval: 15s
  tracing:
    enabled: false  # Future
    endpoint: http://jaeger:14268/api/traces

# Logging
logging:
  level: info  # trace, debug, info, warn, error
  format: json  # json or pretty
  output: stdout  # stdout or file path

# Security
security:
  api_key_required: true
  rate_limit_requests_per_second: 1000
  max_request_body_size: 104857600  # 100MB
```

## Health Checks

### Simple Health Check

```bash
curl http://localhost:9200/_cluster/health
```

Response (green = healthy):
```json
{
  "cluster_name": "lexum-prod",
  "status": "green",
  "number_of_nodes": 1,
  "active_primary_shards": 5,
  "active_shards": 5,
  "relocating_shards": 0,
  "initializing_shards": 0,
  "unassigned_shards": 0
}
```

### Extended Health Check

```bash
curl http://localhost:9200/_cluster/stats
```

Provides index count, document count, storage size, request rates.

## Monitoring & Metrics

### Prometheus Endpoint

```bash
curl http://localhost:9200/_metrics
```

Returns Prometheus-format metrics:
```
lexum_http_requests_total{method="POST",status="200"} 1234
lexum_http_request_duration_seconds_bucket{endpoint="/search",le="0.1"} 945
lexum_index_document_count{index="products"} 1000000
lexum_index_size_bytes{index="products"} 536870912
```

### Key Metrics to Monitor

| Metric | Alert Threshold | Action |
|--------|-----------------|--------|
| `http_requests_total{status="5xx"}` | >1% | Check server logs |
| `http_request_duration_seconds{p95}` | >1s | Profile query load |
| `index_size_bytes` | >disk/2 | Plan storage expansion |
| `nodes_active` | <cluster_size | Investigate node failure |
| `unassigned_shards` | >0 | Trigger shard rebalancing |

### Grafana Dashboard

Import dashboard ID `TBD` (planned) for visualization of key metrics.

## Backups & Snapshots

### Create Snapshot

```bash
# CLI
lexum snapshot create backup snap_2024 --indices products --wait

# API
curl -X POST http://localhost:9200/api/v1/snapshots \
  -H "X-API-Key: api-key" \
  -d '{"name": "snap_2024", "indices": ["products"]}'
```

### List Snapshots

```bash
lexum snapshot list
# or
curl http://localhost:9200/api/v1/snapshots
```

### Restore from Snapshot

```bash
lexum snapshot restore snap_2024 --indices products

# or
curl -X POST http://localhost:9200/api/v1/snapshots/snap_2024/restore
```

## Troubleshooting

### Server Won't Start

**Error**: "Address already in use"
```bash
# Kill process on port 9200
lsof -i :9200
kill -9 <pid>
```

**Error**: "Invalid argument" on index creation (WSL)
```bash
# Solution: Use Windows native paths, not WSL /mnt/ paths
# Or store in WSL native home (~/.lexum/data)
```

### Slow Queries

```bash
# Enable debug logging
RUST_LOG=debug ./lexum-server

# Check query execution time in metrics
curl http://localhost:9200/_metrics | grep http_request_duration

# Use query explain endpoint
curl -X GET "http://localhost:9200/api/v1/search/explain" \
  -d '{"query": {...}}'
```

### High Memory Usage

**Action**: Reduce cache sizes in config.yml
```yaml
search:
  query_cache_size: 100000  # was 1000000
  field_cache_size: 10000000  # was 100000000
```

### Cluster Health Red

```bash
# Check status
curl http://localhost:9200/_cluster/health

# Check node status
curl http://localhost:9200/_nodes

# Check unassigned shards (in multi-node)
curl http://localhost:9200/_cluster/stats
```

## Performance Tuning

### Index Writing

```yaml
search:
  merge_factor: 20  # More frequent merges = better search, slower writes
  buffer_size_mb: 256  # Larger buffer = faster writes, more memory
```

### Query Execution

```yaml
search:
  index_cache_size: 1000000  # Cache popular queries
  field_cache_size: 100000000  # Cache sorting/aggregation fields
```

### Concurrency

```bash
# Set worker threads (default: num_cpus)
LEXUM_WORKER_THREADS=16 ./lexum-server
```

## Security Best Practices

1. **Enable Authentication**: Set `LEXUM_API_KEY_REQUIRED=true`
2. **Use TLS** (future): Encrypted inter-node communication
3. **Rate Limiting**: Configure per-API-key limits
4. **Firewall**: Restrict port 9200 to trusted networks
5. **Audit Logging**: Monitor access via structured logs
6. **Snapshots**: Store backups in secure location (S3 with encryption)
