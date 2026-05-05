# Umicp — Cortex Ingestion Priorities

## Why Cortex Needs Umicp Integration

**Context:** Cortex is the indexing/consolidation layer for all HiveHub projects. Umicp is the communication protocol underlying Vectorizer, Nexus, and other services.

**Goal:** Index Umicp capabilities for discovery and usage in Cortex workflows.

## Ingestion Strategy

### Priority 1: Protocol Specification (Critical)

**What to Index:**
- Envelope schema (JSON structure, required/optional fields)
- OperationType enum (DATA, CONTROL, ACK, ERROR, HEARTBEAT)
- PayloadType enum (VECTOR, MATRIX, TENSOR, JSON, BINARY, TEXT)
- Serialization formats (CBOR, JSON, compression algorithms)

**How Cortex Uses It:**
- Validate message schemas in-flight
- Route messages based on operation type
- Transform payloads between formats
- Detect incompatibilities between versions

**Priority Level:** 🔴 CRITICAL (Day 1)

### Priority 2: SDK Capabilities Matrix (High)

**What to Index:**
- Per-language feature availability (10 SDKs)
- Transport support (WebSocket, HTTP/2)
- Matrix operation support (SIMD yes/no)
- Compression algorithms per language
- Performance characteristics

**How Cortex Uses It:**
- Recommend optimal SDK for use case
- Detect feature gaps when choosing SDK
- Plan fallbacks for unavailable features
- Track performance SLAs

**Priority Level:** 🟠 HIGH (Week 1)

### Priority 3: Integration Patterns (High)

**What to Index:**
- Request-response pattern (sendAndWait)
- Broadcast pattern (broadcast to multiple peers)
- Pub-sub event system
- Service discovery protocol
- Multiplexed peer handshake

**How Cortex Uses It:**
- Recommend correct pattern for use case
- Generate integration examples
- Validate architectural decisions
- Track coupling between services

**Priority Level:** 🟠 HIGH (Week 1)

### Priority 4: Configuration Schema (Medium)

**What to Index:**
- UMICPConfig options (max_message_size, timeouts, encryption)
- Per-language configuration methods
- Environment variables
- Defaults and recommended values
- Trade-offs for each option

**How Cortex Uses It:**
- Auto-generate configuration for deployments
- Detect configuration conflicts
- Recommend tuning for performance/security
- Track non-standard configurations

**Priority Level:** 🟡 MEDIUM (Week 2)

### Priority 5: Operational Data (Medium)

**What to Index:**
- Default ports (20081, 8080, etc.)
- Performance benchmarks (msg/sec, latency)
- Resource requirements (memory per connection)
- Scaling limits (tested to 10K connections)
- Monitoring metrics (key metrics to track)

**How Cortex Uses It:**
- Capacity planning for new deployments
- Performance regression detection
- Benchmark new code changes
- Estimate resource allocation

**Priority Level:** 🟡 MEDIUM (Week 2)

### Priority 6: Compatibility Matrix (Low)

**What to Index:**
- Cross-version compatibility (v0.2 ↔ v0.3)
- Cross-language feature parity
- Transport compatibility (WebSocket ↔ HTTP/2)
- Framework integration support

**How Cortex Uses It:**
- Version upgrade planning
- Migration guidance (v0.2 → v0.3)
- Detect downstream breaking changes
- Track deprecation status

**Priority Level:** 🟢 LOW (Week 3)

## Data Integration Points

### With Vectorizer

**Umicp Role:** Transport for embedding submission/retrieval

**Integration in Cortex:**
- Index embeddings sent via Umicp to Vectorizer
- Track which SDKs/languages submit embeddings
- Monitor throughput (embeddings/sec)
- Detect schema mismatches

**Sample Query:** "Show me all embeddings submitted to Vectorizer via Umicp in the last 24 hours, grouped by sender"

### With Nexus

**Umicp Role:** Internal service-to-service communication

**Integration in Cortex:**
- Index external ID mappings (phase11l feature)
- Track node creation/update operations
- Monitor graph traversal queries
- Detect topology changes

**Sample Query:** "Which services are talking to Nexus? How often?"

### With Task Queue

**Umicp Role:** Worker-to-coordinator communication

**Integration in Cortex:**
- Index task assignments (broadcast messages)
- Track worker status updates (CONTROL messages)
- Monitor task completion (ACK pattern)
- Detect stalled workers

**Sample Query:** "Which workers haven't reported status in 10 minutes?"

### With Agent Framework

**Umicp Role:** Agent-to-agent peer communication

**Integration in Cortex:**
- Index peer connections and topology
- Track agent capabilities (capability negotiation)
- Monitor agent message patterns
- Detect agent discovery failures

**Sample Query:** "Show me the peer network topology—which agents are connected to which?"

## Cortex-Specific Ingestion Tasks

### Task 1: Schema Ingestion

**Input:** Envelope schema from protocol spec

**Processing:**
- Parse JSON schema
- Extract field types and constraints
- Identify required vs. optional
- Mark version (0.3.0)

**Output:** Cortex schema document with validation rules

**Source Files:** 
- `docs/guides/protocol-api.md`
- `bindings/*/docs/GUIDE.md`

### Task 2: Capability Scanning

**Input:** All 10 SDK source files

**Processing:**
- Extract feature flags per language
- Test build each SDK
- Run test suites
- Collect performance benchmarks

**Output:** Capability matrix (SDK x Feature) with status/metrics

**Source Files:**
- `bindings/{python,rust,typescript,go,csharp,php,elixir,java,kotlin,swift}`

### Task 3: Integration Pattern Mapping

**Input:** Examples and documentation

**Processing:**
- Extract code patterns (request-response, broadcast, etc.)
- Identify when each pattern is used
- Map to use cases (ML training, service discovery, etc.)
- Note pitfalls/gotchas

**Output:** Pattern library with examples per SDK

**Source Files:**
- `bindings/*/examples/`
- `docs/guides/*`

### Task 4: Configuration Template Generation

**Input:** Config schema from C++ code + per-SDK docs

**Processing:**
- Generate config templates per SDK language
- Add comments with defaults/recommendations
- Create per-use-case templates (ML training, web service, etc.)

**Output:** Configuration template library

**Source Files:**
- `cpp/include/umicp/config.h`
- `bindings/*/docs/GUIDE.md`

### Task 5: Performance Profile Indexing

**Input:** Benchmarks and test results

**Processing:**
- Parse benchmark output
- Normalize metrics (msg/sec, latency_ms, throughput_mbps)
- Index by SDK, message size, transport
- Track over time for regression detection

**Output:** Performance database with trend analysis

**Source Files:**
- `bindings/*/benchmarks/`
- Test output artifacts

## Output Format for Cortex

### Schema Document

```yaml
kind: Protocol/Schema
name: umicp-envelope-v0.3.0
version: 0.3.0
fields:
  - name: from
    type: string
    required: true
    description: "Sender node ID"
  - name: to
    type: string
    required: true
    description: "Recipient node ID"
  # ... more fields
payload_types:
  - VECTOR
  - MATRIX
  # ... more types
```

### Capability Matrix Document

```yaml
kind: Feature/CapabilityMatrix
sdks:
  python:
    v0.3.2:
      websocket: supported
      http2: supported
      multiplexed_peer: supported
      encryption: supported
      # ...
  rust:
    v0.3.1:
      # ...
```

### Integration Pattern Document

```yaml
kind: Integration/Pattern
name: request-response-rpc
aliases: [sendAndWait, blocking-call]
description: "Synchronous request-response communication"
sdks:
  python: "peer.send_and_wait(peer_id, request, timeout_ms)"
  rust: "peer.send_and_wait(&peer_id, request, timeout_ms).await"
  # ...
use_cases:
  - Model inference queries
  - Service metadata lookup
  - Configuration sync
pitfalls:
  - Timeout may occur on overloaded peer
  - Network latency affects response time
examples:
  - file: bindings/rust/examples/peer_with_handshake.rs
    lines: 45-67
```

## Success Criteria

- ✅ All protocol schemas indexed and queryable
- ✅ Capability matrix complete (10 SDKs x 20+ features)
- ✅ 5+ integration patterns documented with examples
- ✅ Configuration templates for 5 common use cases
- ✅ Performance benchmarks indexed with trending
- ✅ Cross-references to Vectorizer, Nexus, other Hive projects
- ✅ Cortex search queries return relevant Umicp results
