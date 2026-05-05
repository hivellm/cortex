# Umicp Open Questions and Gaps

## Known Limitations

### C++ Feature Parity with TypeScript

**Gap:** Event-driven architecture available in TypeScript SDK (v0.3.1+), not yet in C++ core

**Impact:**
- C++ users must use low-level callbacks
- Multiplexed peer mode (simultaneous server+client) not available in C++
- Advanced broadcast patterns not available

**Timeline:** C++ planned phases 1-3 (18-25 weeks, estimated 6 months)

**Workaround:** Use TypeScript SDK via Node.js FFI or wait for C++ implementation

### Distributed Tracing

**Gap:** No built-in OpenTelemetry integration

**Current State:** Application must track correlation IDs manually

**Use Case:**
```
Request: Client → Service A → Service B → Service C
Problem: Tracing distributed call chain across services
Current: Manual correlation ID passing
Needed: Automatic trace context propagation
```

**Options:**
1. Add W3C Trace Context header support
2. Integrate with OTel SDK per language
3. Middleware for automatic context injection

**Question:** Should Umicp carry trace context in envelope, or is app-level handling sufficient?

### Message Persistence and Offline Queueing

**Gap:** No built-in message queue for offline scenarios

**Current State:** Messages lost if peer is offline

**Use Case:**
```
Agent A (offline) ← Message from Agent B
Problem: Message discarded, no notification of failure
Current: Application must implement retry/queue
Needed: Optional persistent queue
```

**Options:**
1. Plugin-based queue (file, Redis, RabbitMQ)
2. Built-in SQLite queue
3. Require external queue (e.g., Task Queue service)

**Question:** Should Umicp include persistent queueing, or is it application responsibility?

### Schema Validation and Evolution

**Gap:** No built-in schema registry or validation

**Current State:** Enum types and capability negotiation only

**Use Case:**
```
Service: "I accept VECTOR of float32[1536]"
Client: Sends VECTOR of float32[768]
Problem: Runtime error, no early validation
Needed: Schema definition and validation
```

**Options:**
1. JSONSchema or Protocol Buffers integration
2. Custom Umicp schema format
3. Rely on application-layer validation

**Question:** Add schema registry, or keep Umicp minimal?

### Cross-Language Exact Numeric Compatibility

**Gap:** Floating-point precision varies across languages

**Current State:**
- C++ uses native floats (platform-dependent)
- Python uses NumPy (platform-dependent)
- Rust uses std::f32/f64 (IEEE 754)
- JavaScript loses precision for large integers

**Use Case:**
```
Model trained in C++: weights [1.23456789012345]
Deployed in Python: received [1.2345678806...] (precision loss)
Problem: Slightly different inference results
```

**Options:**
1. Enforce fixed-point representation (multiply by 10^n)
2. Use Decimal type for payloads
3. Document as acceptable difference
4. Custom codec per use case

**Question:** What precision guarantees does Umicp make?

### Flow Control and Backpressure

**Gap:** No built-in flow control mechanism

**Current State:** Message queue unbounded (memory-limited)

**Use Case:**
```
Slow consumer receiving 100K msg/sec from fast producer
Problem: Memory exhaustion, message loss
Current: Queue grows until OOM
Needed: Backpressure signal (slow down sender)
```

**Options:**
1. Add QueueFull error response
2. Implement congestion control (TCP-like)
3. Require application-level rate limiting

**Question:** Should protocol enforce backpressure, or application?

### Payload Size Limits

**Gap:** Default 1MB max message size, but no guidance for distributed matrices

**Current State:**
- Small embeddings (1-10KB): Fine
- Model weights (10MB-1GB): Requires chunking or HTTP streaming

**Use Case:**
```
Deploy 1GB model weights
Current: Must chunk into 1MB pieces, track reassembly
Needed: Automatic chunking or streaming
```

**Options:**
1. Transparent chunking (hidden from app)
2. Streaming transport option (HTTP/2 only)
3. Larger default max_message_size per language

**Question:** What's the recommended pattern for >1GB payloads?

### Security Model Clarity

**Gap:** Optional encryption creates two security models

**Current State:**
- TLS transport: Data encrypted in-flight
- Optional Umicp encryption: Application-level crypto
- No clear guidance on when to use which

**Use Case:**
```
Internal network (no TLS): Enable Umicp encryption? 
Public network: Assume TLS sufficient?
Key management: Where do encryption keys come from?
```

**Options:**
1. Mandate TLS for all transports
2. Provide key management infrastructure
3. Document security model per deployment pattern

**Question:** What is the intended security posture?

### Capability Mismatch Handling

**Gap:** No standard behavior when capability mismatch occurs

**Current State:** Capability negotiation at handshake, but behavior on missing feature undefined

**Use Case:**
```
Service A has encryption=true
Service B has encryption=false
Behavior: Allow connection? Block? Degrade?
```

**Options:**
1. Define standard fallback behaviors
2. Allow application handler for mismatch
3. Stricter capability enforcement (fail if mismatch)

**Question:** What's the policy for mismatched capabilities?

### Monitoring and Observability

**Gap:** Limited built-in observability

**Current State:** Basic stats (msg count, latency), no structured logging

**Use Case:**
```
Debug: "Why did message X go to service A instead of B?"
Current: Check app logs manually
Needed: Structured event logging with routing decisions
```

**Options:**
1. Structured logging facade (log4j-like)
2. Built-in tracing for routing decisions
3. Integration with OpenTelemetry

**Question:** What level of built-in observability is needed?

### Cross-SDK Compatibility Testing

**Gap:** Limited cross-language integration tests

**Current State:** Each SDK has tests, but limited TS↔Rust, Python↔Go tests

**Use Case:**
```
Deploy TypeScript frontend + Rust backend
Testing: Do they communicate correctly? Binary compatibility?
Current: Manual testing
Needed: Automated cross-SDK tests
```

**Question:** Should there be a cross-SDK test suite in CI?

## Future Enhancements Under Discussion

1. **Umicp Compression Benchmarking**: Measure actual compression ratios per algorithm, message size
2. **Performance Optimization**: SIMD matrix operations in Python/Go bindings
3. **Kubernetes Native**: Helm charts, service mesh integration (Istio)
4. **GraphQL API**: Query Umicp messages and topology
5. **Web Dashboard**: Visual peer network, message tracing, performance graphs
6. **Umicp Router Service**: Standalone router for complex topologies
7. **Streaming Large Files**: Built-in support for >1GB payloads over HTTP/2
8. **Smart Caching**: Memoize requests, cache invalidation protocol
9. **Load Test Tool**: `umicp-loadtest` CLI for benchmarking
10. **Governance**: RFC process for breaking changes, proposal reviews

## Questions for the Project

### Design Questions

1. Is Umicp intended to be *the* inter-service protocol for all HiveHub, or coexist with gRPC/AMQP?
2. Should Umicp eventually move from library to microservice (centralized router)?
3. What's the target audience: ML researchers, backend engineers, DevOps?

### Operational Questions

1. How are Umicp upgrades coordinated across 10 SDKs? Monthly? Quarterly?
2. What's the support window for old versions (v0.2, etc.)?
3. Should there be an official "Umicp Compatibility Badge" for frameworks/tools?

### Security Questions

1. Are there threat models documented (man-in-the-middle, replay attacks, etc.)?
2. Should encryption be mandatory by default instead of optional?
3. Who manages encryption keys in distributed deployments (agents vs. centralized)?

### Performance Questions

1. What are realistic throughput targets (msg/sec) per SDK?
2. How does performance scale with payload size (linear, sublinear)?
3. Are there benchmarks against alternative protocols (gRPC, AMQP, plain HTTP/2)?
