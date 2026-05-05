# Umicp Data and Storage

## Message Schemas

### Envelope (Outer Message Container)

**Core Fields:**
- `from` (string): Sender node identifier
- `to` (string): Recipient node identifier (or broadcast pattern)
- `operation` (enum): Message type
- `messageId` (UUID): Unique message identifier
- `correlationId` (UUID): For request-response tracking
- `timestamp` (int64): Unix milliseconds
- `capabilities` (object): Feature negotiation

**Payload Fields:**
- `payloadHint` (object): Type and encoding metadata
- `payload` (bytes/json): Actual message data
- `payloadSize` (uint32): Size in bytes

**Optional Fields:**
- `encryptionKey` (string): For encrypted messages
- `signature` (string): ECC digital signature
- `compressionType` (enum): GZIP, BROTLI, LZ4, NONE

### Payload Types

**VECTOR (Embedding):**
- Single-dimensional array of floats
- Typical use: Word embeddings, semantic vectors
- Serialization: Binary array or JSON array

**MATRIX (Model Weights/Gradients):**
- 2D array of floats [rows][cols]
- Typical use: Neural network weights, layer outputs
- Serialization: Binary matrix or JSON 2D array

**TENSOR (Multi-dimensional):**
- N-dimensional array
- Typical use: Batch processing, multi-layer data
- Serialization: Flattened binary + shape metadata

**JSON (Structured Data):**
- Arbitrary JSON object
- Typical use: Configuration, metadata, status
- Serialization: UTF-8 JSON

**BINARY (Raw Bytes):**
- Untyped binary data
- Typical use: Custom serialization, compressed data
- Serialization: Base64 in JSON, raw in binary

**TEXT (String):**
- UTF-8 text
- Typical use: Prompts, logs, messages
- Serialization: String in JSON, UTF-8 in binary

## Serialization Formats

### Binary Serialization (CBOR)

**Advantages:**
- Compact (50-70% smaller than JSON)
- Type-preserving
- Streaming support
- Faster encoding/decoding

**Format:**
- Major type (3 bits) + additional info (5 bits)
- Variable-length integers
- Indefinite-length arrays/maps for streaming

**Use Cases:** High-throughput communication, large matrices, real-time systems

### JSON Serialization

**Advantages:**
- Human-readable
- Universal tooling support
- Debugging-friendly
- Cross-language compatibility

**Format:** RFC 7159 canonical JSON with ordered keys

**Use Cases:** Configuration, metadata, logging, framework integration

### Compression

**Supported Algorithms:**
- **Gzip**: Standard compression (default), CPU: moderate, Ratio: 40-60%
- **Brotli**: Higher compression, slower, Ratio: 50-70%
- **LZ4**: Fast compression, lower ratio, Ratio: 30-50%
- **None**: No compression

**Decision Logic:**
- <1KB: Typically not compressed (overhead exceeds benefit)
- 1-64KB: Gzip recommended
- >64KB: Brotli for storage, LZ4 for streaming

## Data at Rest

### Node-Local Storage

Nodes typically don't persist messages; Umicp is stateless and connectionless. However:

**Optional Persistence:**
- Message logging for audit trails
- Queue persistence for offline queueing
- Event logs for debugging

**Storage Format:** Typically JSON or binary CBOR in files/databases

### Message Routing State

**In-Memory Only:**
- Connection cache
- Peer registry
- Message correlation table
- Statistics counters

**Lifetime:** Connection lifetime; cleared on disconnect/shutdown

## Data in Transit

### Encryption

**Default (Unencrypted):** Binary CBOR on TLS/WebSocket

**With Encryption:**
1. Plaintext envelope → encrypt payload
2. AES-256-GCM symmetric encryption
3. Key management via security manager

**TLS/SSL Support:**
- WebSocket over TLS (wss://)
- HTTP/2 over TLS (https://)

### Message Ordering

**Delivery Guarantees:**
- **At-most-once:** No retransmission (default)
- **At-least-once:** Can implement with ack pattern
- **Exactly-once:** Requires app-level deduplication

**Ordering:** FIFO per connection, no global ordering

## Protocol Buffers / Schema Evolution

**No Built-in Schema Registry:** Umicp relies on capability negotiation instead

**Capability System:**
```json
"capabilities": {
  "version": "0.3.0",
  "features": ["compression", "encryption", "multiplexed-peer"],
  "custom": { "max_matrix_size": 10000 }
}
```

**Versioning Strategy:**
- Sender advertises capabilities
- Receiver checks compatibility
- Graceful fallback to lower feature set

**Breaking Changes:**
- New operation types added (backward compatible)
- New payload types added (backward compatible)
- Envelope structure changes (requires version bump)

## Resource Constraints

**Per-Node Defaults:**
- Max message size: 1MB (configurable)
- Message queue: Unbounded (memory-limited)
- Max connections: Configurable (tested to 10K)
- Frame buffer: ~100 bytes overhead per message

**Scaling Considerations:**
- Matrix size O(n²) memory impact
- Connection pools scale linearly
- Event listeners scale per registered handler
