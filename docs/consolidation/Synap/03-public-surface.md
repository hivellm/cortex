# Synap — Public Surface

## REST API Endpoints

### Key-Value
- `POST /kv/set` — Set key (with TTL)
- `GET /kv/get/{key}` — Get value
- `DELETE /kv/delete/{key}` — Delete key
- `POST /kv/mset` — Batch set
- `POST /kv/mget` — Batch get
- `POST /kv/incr/{key}` — Increment
- `GET /kv/keys` — List keys (with pattern)

### Queue
- `POST /queue/{queue_name}` — Create queue
- `POST /queue/{queue_name}/publish` — Publish message
- `GET /queue/{queue_name}/consume/{consumer_id}` — Consume
- `POST /queue/{queue_name}/ack` — Acknowledge
- `GET /queue/{queue_name}/stats` — Queue stats

### Stream
- `POST /stream/publish` — Publish to room
- `GET /stream/subscribe/{room}` — Subscribe (SSE)
- `GET /stream/history/{room}` — Get history

### Pub/Sub
- `POST /pubsub/publish` — Publish to topic
- `GET /pubsub/subscribe/{topic}` — Subscribe (SSE/WebSocket)

## SDKs

### Official SDKs
- **Rust**: `synap-sdk` (v0.12.0, async, RxJS-style reactive)
- **TypeScript**: Node.js/browser, full protocol support
- **Python**: Async/sync clients
- **PHP, C#, Go, Java**: Multi-language support

### Protocol Support
All SDKs select transport via URL scheme (no builder flags):
```rust
// Rust
let client = SynapClient::new("synap://127.0.0.1:15501").await?;

// TypeScript
const client = new SynapClient("synap://127.0.0.1:15501");

// Python
client = SynapClient(SynapConfig("synap://127.0.0.1:15501"))
```

## CLI

**synap-cli** — Interactive REPL for manual testing and administration
```bash
synap> SET key value
synap> GET key
synap> QUEUE list  # List queues
```

## Special Protocols

- **MCP (Model Context Protocol)**: Configurable tools for Cursor, Claude Desktop
- **UMICP**: Universal Matrix Inter-Communication Protocol (13 ops, TLS)
- **WebSocket**: Persistent connections for real-time subscriptions
- **StreamableHTTP**: Custom streaming protocol (alternative to WebSocket)
