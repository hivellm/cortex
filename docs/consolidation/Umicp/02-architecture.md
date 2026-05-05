# Umicp Architecture

## Protocol Layers

### Layer 1: Envelope (Message Format)

Core messaging container with metadata:
- **from**: Sender node ID
- **to**: Recipient node ID
- **operation**: Message type (DATA, CONTROL, ACK, ERROR, HEARTBEAT)
- **messageId**: Unique identifier for correlation
- **capabilities**: Capability negotiation for peer features
- **payload**: Binary or JSON data
- **timestamp**: Creation time
- **correlationId**: For request-response patterns

### Layer 2: Transport Layer

**WebSocket Transport:**
- Bidirectional real-time communication
- Client/server and peer-to-peer modes
- Auto-reconnect with exponential backoff
- Heartbeat mechanism for keep-alive
- Connection pooling for multiple peers

**HTTP/2 Transport:**
- Request/response pattern over HTTP/2 (axum 0.8 in Rust, libcurl in C++)
- Header compression
- Multiplexed streams
- Streaming support for large payloads

**Custom Transports:** Framework-agnostic abstraction allows pluggable implementations

### Layer 3: Serialization

**Binary Format (CBOR):**
- Compact encoding for efficient transmission
- Type-safe serialization
- Support for nested structures
- Compression support (Gzip, Brotli, LZ4)

**JSON Format:**
- Human-readable alternative
- Native JSON type support (v0.2.0+)
- Cross-language compatibility

### Layer 4: Security

**Encryption:**
- AES-256 encryption for sensitive data
- Optional per-message or per-connection encryption

**Authentication:**
- ECC (Elliptic Curve Cryptography) digital signatures
- Two-way peer authentication in handshakes
- Message verification

**Key Exchange:**
- Secure keypair generation
- Capability-based access control

## Runtime Architecture

### Protocol Instance

Main entry point managing:
- Configuration (message sizes, timeouts, security requirements)
- Transport lifecycle (connect/disconnect)
- Message routing and queuing
- Statistics collection

### Envelope Flow

```
Send Path:
  App → Envelope Builder → Serialization → Transport → Network

Receive Path:
  Network → Transport → Deserialization → Handler Callbacks → App
```

### Multiplexed Peer (Advanced)

Simultaneous server and client in one instance:
- Peer ID uniquely identifies node
- Inbound connections (remote peers connecting to you)
- Outbound connections (you connecting to remote peers)
- Broadcasting to multiple connected peers
- Request-response pattern with timeout

### Connection Management

- **Connection Pool**: Reuses established connections
- **Load Balancing**: Round-robin or least-connections strategies
- **Failover**: Automatic transport fallback
- **Service Discovery**: Peer service registry and lookup

## Message Types (OperationType)

- **DATA**: Payload transmission (embeddings, model weights)
- **CONTROL**: Protocol-level coordination (status, requests)
- **ACK**: Delivery confirmation
- **ERROR**: Error reporting
- **HEARTBEAT**: Keep-alive ping
- **HANDSHAKE**: Peer capability negotiation

## Payload Types

- **VECTOR**: Single vector embedding
- **MATRIX**: 2D matrix (model weights, gradients)
- **TENSOR**: Multi-dimensional tensor
- **JSON**: Arbitrary JSON structure
- **BINARY**: Raw binary data
- **TEXT**: UTF-8 text

## Error Handling

Result<T> pattern for fallible operations:
- Success variant with T
- Error variant with code + message
- No exceptions for protocol errors
- Graceful degradation on transport failure
