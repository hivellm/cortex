# Umicp Public Surface

## Protocol Specification

### Envelope Schema (JSON)

```json
{
  "from": "node-id",
  "to": "target-node-id",
  "operation": "DATA|CONTROL|ACK|ERROR|HEARTBEAT",
  "messageId": "uuid-v4",
  "correlationId": "uuid-v4",
  "capabilities": {
    "version": "0.3.0",
    "features": ["compression", "encryption"]
  },
  "payloadHint": {
    "type": "VECTOR|MATRIX|TENSOR|JSON|BINARY|TEXT",
    "encoding": "binary|json"
  },
  "payload": "<binary or json>",
  "timestamp": 1696123456789
}
```

### Core Methods

**Connection Management:**
- `connect()` - Establish transport connection
- `disconnect()` - Close gracefully
- `is_connected()` - Connection status

**Sending:**
- `send_data(to, data)` - Send binary/payload
- `send_control(to, message)` - Send control message
- `send_ack(to, msgId)` - Acknowledge received message
- `send_error(to, error)` - Send error message

**Configuration:**
- `configure(config)` - Set protocol options
- `set_transport(transport)` - Choose transport backend
- `set_security_manager(security)` - Enable encryption/signing

**Monitoring:**
- `get_stats()` - Performance metrics (messages sent/received, latency, throughput)
- `reset_stats()` - Clear statistics

**Multiplexed Peer (Advanced):**
- `connectToPeer(url)` - Establish peer connection
- `broadcast(envelope)` - Send to all connected peers
- `broadcastToType(envelope, type)` - Selective broadcast
- `sendAndWait(peerId, envelope, timeout)` - RPC-style request-response

## SDK Availability

### Published SDKs

| Language | Package | Version | Install |
|----------|---------|---------|---------|
| Python | `umicp_sdk` | 0.3.2 | `pip install umicp-sdk` |
| Rust | `umicp-sdk` | 0.3.1 | `cargo add umicp-sdk` |
| TypeScript | `@hivehub/umicp-sdk` | 0.3.1 | `npm install @hivehub/umicp-sdk` |
| C# | `HiveHub.Umicp.SDK` | 0.3.0 | `dotnet add HiveHub.Umicp.SDK` |
| PHP | `hivehub/umicp-sdk` | 0.3.0 | `composer require hivehub/umicp-sdk` |
| Elixir | `umicp` | 0.3.0 | Add to `mix.exs` |
| Go | `github.com/hivehub/umicp-sdk` | 0.3.0 | `go get ...` |
| Swift | `UMICP-SDK` | 0.3.0 | SPM / Package.swift |
| Kotlin | `umicp-sdk` | 0.3.0 | Maven/Gradle ready |
| Java | `umicp-sdk` | 0.3.0 | Maven/Gradle ready |

### Feature Matrix by Language

| Feature | Python | Rust | TS | Go | C# | PHP | Elixir | Java | Kotlin | Swift |
|---------|--------|------|----|----|----|----|--------|------|--------|-------|
| Core Protocol | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| WebSocket | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| HTTP/2 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Multiplexed Peer | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Event System | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Encryption | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Compression | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ |
| SIMD Matrix | ✅ (NumPy) | ✅ (ndarray) | ✅ (C++ FFI) | ❌ | ✅ | ✅ (C++ FFI) | ⚠️ | ❌ | ❌ | ✅ |
| Service Discovery | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

## CLI Tools

### MCP Bridge (Model Context Protocol)

Connect Umicp to LLM IDEs (Cursor, etc.):

```bash
npm install -g @hivehub/umicp2mcp
npx @hivehub/umicp2mcp
```

**Cursor Configuration (.cursor/mcp.json):**
```json
{
  "mcpServers": {
    "umicp": {
      "command": "npx",
      "args": ["@hivehub/umicp2mcp"]
    }
  }
}
```

**Usage:** Execute UMICP operations directly from Cursor's AI context.

## Configuration Schema

```cpp
struct UMICPConfig {
  size_t max_message_size;      // Default: 1MB
  uint32_t connection_timeout;  // Default: 5000ms
  bool enable_binary;           // Default: true
  ContentType preferred_format; // Default: BINARY
  bool require_auth;            // Default: false
  bool require_encryption;      // Default: false
};
```

## Statistics Structure

```cpp
struct ProtocolStats {
  uint64_t messages_sent;
  uint64_t messages_received;
  uint64_t bytes_sent;
  uint64_t bytes_received;
  uint64_t errors;
  uint64_t timeouts;
  double avg_latency_ms;
  double throughput_mbps;
  timestamp last_activity;
};
```

## Performance Characteristics

- **Small Messages (<1KB)**: >10,000 msg/sec
- **Medium Messages (1KB-64KB)**: >1,000 msg/sec
- **Large Messages (>64KB)**: >1GB/sec throughput
- **Local Network Latency**: <1ms average
- **Processing Overhead**: <0.1ms per message
- **Concurrent Connections**: Tested up to 10,000
