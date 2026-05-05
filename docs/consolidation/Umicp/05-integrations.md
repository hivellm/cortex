# Umicp Integrations

## HiveHub Projects Using Umicp

### Vectorizer (Semantic Search Engine)

**Integration Method:** Umicp agent protocol for embedding storage/retrieval

**Endpoints:**
- `/umicp` - Umicp service discovery and messaging
- Custom endpoint support for compatibility

**Communication Pattern:**
- Agents submit embeddings via Umicp DATA messages
- Vectorizer returns search results via CONTROL messages
- Pipeline: Models → Umicp → Vectorizer → Vector DB

**SDK Support:** All 10 language bindings supported

### Nexus (Graph Database)

**Integration Method:** Internal service-to-service communication

**Operations:**
- Node operations (create, read, update, delete)
- Edge traversal
- External ID integration (phase11l feature)

**Communication:** Bidirectional WebSocket/HTTP/2 transport

### Task Queue (Workflow Orchestration)

**Integration Method:** Distributed task coordination via Umicp peers

**Pattern:** 
- Workers register as Umicp peers
- Task scheduler broadcasts work assignments
- Workers report status via CONTROL messages
- Result aggregation via DATA messages

### Agent Framework

**Integration Method:** Peer-to-peer communication between agents

**Features:**
- Multiplexed peer mode (simultaneous server+client)
- Event-driven message handling
- Service discovery for agent registry
- Broadcast for multi-agent coordination

**Communication:** WebSocket for real-time, HTTP/2 for reliability

### Voxa (Voice AI Assistant)

**Integration Method:** Agent coordination for multi-modal interaction

**Pipeline:**
- Speech → ASR → NLU → Agent Inference → Voice Synthesis
- Inter-stage communication via Umicp
- Real-time streaming over WebSocket

## Cross-Project Communication Patterns

### Synchronous Request-Response

```
Client → send_and_wait(server, request, 5000ms)
       ↓ (WebSocket connection)
Server → receives envelope
Server → processes
Server → send_data(client, response)
Client ← receives response with correlationId
```

**SDK Method:** `peer.sendAndWait()` (all languages)

### Asynchronous Publish-Subscribe

```
Publisher → broadcast(envelope, type=OUTGOING)
         ↓ (to all connected subscribers)
Subscriber1 → message event
Subscriber2 → message event
Subscriber3 → message event
```

**SDK Method:** `peer.broadcast()` with type filtering

### Matrix/Embedding Distribution

```
Producer → send_data(recipient, envelope{payloadType: VECTOR})
        ↓ (binary CBOR, compressed)
Consumer → receives matrix/embedding
Consumer → processes locally
Consumer → sends_ack() if needed
```

**Typical Use:** ML model inference, gradient aggregation

### Service Discovery

```
Service Instance → advertise(service_name, capabilities)
                ↓ (registers in local registry)
Other Components → discover(service_name)
                ↓ (finds available instances)
                → connects to optimal instance
```

## Integration with External Systems

### HTTP Gateway Pattern

```
External Client → HTTP/2 POST /umicp
              ↓ (Umicp HTTP Server)
              → Envelope creation
              → Route to peer
              ↓ (WebSocket)
Umicp Network → processes
              ↓ HTTP response
External Client ← response data
```

**Use Case:** REST API clients accessing Umicp network

### Load Balancing Integration

**Multi-transport Load Balancer:**
- Round-robin across WebSocket/HTTP/2 transports
- Least-connections strategy
- Automatic failover to healthy transport

**Typical Setup:**
1. Multiple Umicp instances
2. Load balancer health checks
3. Automatic transport selection per peer

### MCP (Model Context Protocol) Bridge

**Purpose:** Integrate Umicp operations into LLM IDEs

**Tools Exposed:**
- `echo` - Test connectivity
- `send_message` - Send Umicp message
- `create_envelope` - Build message
- `list_peers` - Service discovery

**Supported IDEs:** Cursor, Claude Code, others with MCP support

## Framework Integration Examples

### Axum (Rust HTTP Server)

```rust
use axum::{Router, Json, extract::State};
use umicp_sdk::{Envelope, Protocol};

let app = Router::new()
    .route("/umicp", post(handle_umicp))
    .with_state(protocol.clone());

async fn handle_umicp(
    State(protocol): State<Arc<Protocol>>,
    Json(envelope): Json<Envelope>
) -> Json<Envelope> {
    // Process Umicp envelope
    protocol.send_data(&envelope.to, envelope.payload).await;
    response_envelope
}
```

### Express.js (Node.js Web Server)

```javascript
const express = require('express');
const { Protocol } = require('@hivehub/umicp-sdk');

const app = express();
const protocol = new Protocol('express-server');

app.post('/umicp', async (req, res) => {
  const envelope = req.body;
  await protocol.sendData(envelope.to, envelope.payload);
  res.json(responseEnvelope);
});
```

### FastAPI (Python)

```python
from fastapi import FastAPI, Body
from umicp_sdk import Protocol, Envelope

app = FastAPI()
protocol = Protocol("fastapi-server")

@app.post("/umicp")
async def handle_umicp(envelope: Envelope):
    await protocol.send_data(envelope.to, envelope.payload)
    return response_envelope
```

## Data Integration Points

### Embedding Storage (Vectorizer)

Umicp transports embeddings to Vectorizer:
- VECTOR payload type for single embeddings
- MATRIX payload type for batches
- Binary CBOR serialization for efficiency

### Model Weight Distribution

Gradient/weight synchronization in federated learning:
- MATRIX payloads for weight matrices
- Compression for large models (>10MB)
- Streaming HTTP/2 for bandwidth efficiency

### Log and Telemetry

Optional integration for monitoring:
- Send protocol stats via CONTROL messages
- Event system for error logging
- Compatible with OpenTelemetry

## Known Integration Challenges

1. **Backward Compatibility:** v0.2 to v0.3 required package renaming across SDKs
2. **Custom Endpoints:** Different services use different Umicp paths (/umicp, /message, /data)
3. **Network Topology:** Firewall/NAT traversal for peer-to-peer connections
4. **Error Propagation:** No built-in distributed tracing (application-level correlation IDs recommended)
