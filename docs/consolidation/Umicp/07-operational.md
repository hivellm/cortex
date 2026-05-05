# Umicp Operational Guide

## Docker Deployment

### Running Umicp Services

**No Official Container:** Umicp is a library, not a standalone service. Deployment pattern:

**Pattern 1: Sidecar Container**
```dockerfile
FROM rust:latest
RUN cargo install umicp-cli
COPY app /app
WORKDIR /app
RUN cargo build --release
CMD ["./target/release/app"]
```

**Pattern 2: Multi-Stage Build**
```dockerfile
# Build C++ core
FROM ubuntu:22.04 as builder
RUN apt-get install -y cmake libssl-dev
COPY . /src
WORKDIR /src/cpp
RUN mkdir build && cd build && cmake .. && make

# Python application using Umicp
FROM python:3.10
COPY --from=builder /src/cpp/build/lib /usr/local/lib
RUN pip install umicp-sdk
COPY app.py /app/
CMD ["python", "/app/app.py"]
```

**Pattern 3: Node.js MCP Bridge**
```dockerfile
FROM node:18
RUN npm install -g @hivehub/umicp2mcp
EXPOSE 3000
CMD ["umicp2mcp", "--port", "3000"]
```

## Network Configuration

### Default Ports

| Service | Default Port | Transport | Use Case |
|---------|--------------|-----------|----------|
| Umicp Peer Server | 20081-20099 | WebSocket | Agent-to-agent |
| Umicp HTTP Server | 8080 | HTTP/2 | REST gateway |
| MCP Bridge | 3000 | HTTP | IDE integration |
| Custom App | Configurable | Any | Application-specific |

### Example Configuration

```rust
// Rust SDK configuration
let peer_config = UMICPPeerConfig {
    peer_id: "my-service".to_string(),
    server: Some(ServerConfig {
        port: 20081,
        path: "/umicp",
        ..Default::default()
    }),
    ..Default::default()
};
```

## Environment Variables

### C++ Core

```bash
UMICP_LOG_LEVEL=debug|info|warn|error|trace
UMICP_MAX_MESSAGE_SIZE=1048576  # 1MB default
UMICP_CONNECTION_TIMEOUT=5000   # milliseconds
UMICP_ENABLE_COMPRESSION=true
UMICP_COMPRESSION_ALGORITHM=gzip|brotli|lz4
```

### Rust SDK

```bash
RUST_LOG=umicp=debug              # Tracing level
UMICP_MAX_CONNECTIONS=10000
UMICP_POOL_SIZE=32
```

### Python SDK

```bash
UMICP_DEBUG=1
UMICP_ASYNC_TIMEOUT=30
```

## Health Checks

### WebSocket Peer Health

**Method 1: Heartbeat Messages**
```rust
protocol.send_control(&peer_id, "heartbeat").await;
// Expects ACK response or timeout
```

**Method 2: Connection Status**
```rust
let connected = protocol.is_connected();
```

**Method 3: Statistics**
```rust
let stats = protocol.get_stats();
if stats.errors > threshold {
    alert_monitoring_system();
}
```

### HTTP/2 Endpoint Health

```bash
# Check if HTTP server is running
curl -v http://localhost:8080/health

# Send test message
curl -X POST http://localhost:8080/umicp \
  -H "Content-Type: application/json" \
  -d '{"from":"test","to":"target","operation":"DATA"}'
```

## Monitoring and Metrics

### Key Metrics to Track

**Connection Metrics:**
- `total_connections` - Active peer connections
- `connection_errors` - Connection failures
- `reconnect_attempts` - Auto-reconnect triggers
- `connection_latency_ms` - Connection establishment time

**Message Metrics:**
- `messages_sent` - Total sent
- `messages_received` - Total received
- `bytes_sent` - Bandwidth usage
- `bytes_received` - Bandwidth usage
- `message_errors` - Send/receive failures
- `avg_latency_ms` - Per-message roundtrip

**Performance Metrics:**
- `throughput_mbps` - Current bandwidth
- `queue_depth` - Pending messages
- `compression_ratio` - Gzip/Brotli effectiveness

**Security Metrics:**
- `encryption_errors` - Crypto failures
- `signature_failures` - Auth errors
- `capability_mismatches` - Version/feature conflicts

### Prometheus Export Format

```
umicp_connections_total{service="my-service"} 42
umicp_messages_sent_total{service="my-service"} 1000000
umicp_message_latency_ms{service="my-service",quantile="p50"} 0.5
umicp_message_latency_ms{service="my-service",quantile="p99"} 2.3
```

## Scaling Considerations

### Vertical Scaling (Single Node)

**Limits Tested:**
- 10,000 concurrent connections per node
- >10,000 messages/sec throughput
- ~50-200 MB memory per 1000 connections

**Optimization:**
- Increase connection pool size: `UMICP_POOL_SIZE=64`
- Adjust message queue: `max_queue_depth=10000`
- Enable compression for >64KB messages

### Horizontal Scaling (Multiple Nodes)

**Load Balancing Pattern:**
```
[Client] → [LB: Round-robin] → [Umicp Node 1]
                              → [Umicp Node 2]
                              → [Umicp Node 3]
```

**Service Discovery:**
- Each node registers in agent framework registry
- Clients discover via `discover("service-name")`
- Automatic failover to next healthy instance

**State Sharing:**
- Stateless design → no session affinity needed
- Peer registry cached locally (lazy sync)
- Message correlation via UUID (client tracks)

## Troubleshooting

### Connection Issues

**Symptom:** Connection timeout

**Diagnostics:**
```bash
# Check if peer is reachable
curl -w "@curl-format.txt" -o /dev/null -s http://peer:8080/health

# Check firewall
netstat -an | grep ESTABLISHED | wc -l

# Enable debug logging
RUST_LOG=debug cargo run
```

**Solutions:**
1. Verify firewall rules (WebSocket port 20081, HTTP port 8080)
2. Check peer address/hostname resolution
3. Increase `connection_timeout` if network is slow

### Message Loss

**Symptom:** Messages not received

**Diagnostics:**
```rust
let stats = protocol.get_stats();
println!("Sent: {}, Received: {}, Errors: {}", 
  stats.messages_sent, stats.messages_received, stats.errors);
```

**Solutions:**
1. Enable ACK pattern for delivery confirmation
2. Check queue depth (may be dropping if full)
3. Verify peer is not overloaded (`avg_latency_ms`)

### High Latency

**Symptom:** avg_latency_ms > acceptable threshold

**Diagnostics:**
```bash
# Network latency test
ping -c 10 peer-address

# Check CPU/memory utilization
top, ps aux

# Check message sizes
RUST_LOG=trace (filters for payload_size)
```

**Solutions:**
1. Enable compression for large messages
2. Scale horizontally (add more peers)
3. Reduce max_message_size if appropriate
4. Use HTTP/2 instead of WebSocket if latency is transport-related

## Backup and Recovery

### Connection State

**Nature:** Stateless; no persistence needed

**Recovery:** Auto-reconnect with exponential backoff (default: 100ms → 30s)

### Message Queue

**Optional Persistence:** Application-level only
```rust
// Manual queue persistence
let pending = protocol.get_pending_messages();
persist_to_disk(&pending);

// On restart
let persisted = load_from_disk();
for msg in persisted {
  protocol.send_data(&msg.to, msg.payload).await;
}
```

### Peer Registry

**Nature:** In-memory, rebuilt on startup

**Resilience:** Each peer re-discovers others on connection failure

## Version Upgrades

### Breaking Changes in v0.3

- Package name standardization (umicp → umicp-sdk)
- All SDKs updated simultaneously
- Capability negotiation allows gradual rollout

### Upgrade Path

1. Deploy new version to subset of nodes
2. Test inter-version communication (backward compatible)
3. Roll out to remaining nodes
4. Monitoring confirms no errors

### Compatibility Matrix

| v0.2 ↔ v0.3 | Feature | Compatible |
|------------|---------|-----------|
| Envelope | Basic structure | ✅ Yes |
| Operations | DATA, CONTROL, ACK | ✅ Yes |
| Payloads | VECTOR, MATRIX, JSON | ✅ Yes |
| Capabilities | New fields | ✅ Yes (ignored if unknown) |
