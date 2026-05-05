# Lexum Design Decisions & Rationale

## Technology Choices

### Rust 2024 Edition
**Decision**: Build entire project in Rust (not polyglot)

**Rationale**:
- Memory safety without GC (eliminates crashes from null pointers, buffer overflows)
- Zero-cost abstractions (final binary as fast as C/C++)
- Fearless concurrency (type system prevents data races)
- Excellent async runtime (Tokio) native to language
- Strong type system catches errors at compile time
- Single binary deployment (no runtime dependencies)

**Trade-off**: Longer compilation times, steeper learning curve

### Tantivy 0.25 Over Custom
**Decision**: Use proven full-text search library vs building from scratch

**Rationale**:
- Lucene-inspired (battle-tested algorithms)
- BM25 scoring built-in
- Active maintenance and community
- Well-documented API
- Performance proven in production

**Trade-off**: Less flexibility for custom indexing strategies

### Tokio + Axum Stack
**Decision**: Async runtime + web framework combo

**Rationale**:
- Industry-standard (used by Cloudflare, Discord, etc.)
- High concurrency without thread proliferation
- Axum provides type-safe routing
- Rich middleware ecosystem (Tower)
- Excellent performance characteristics
- Mature, stable APIs

**Trade-off**: Async complexity, harder debugging

### RocksDB for Metadata
**Decision**: Use embedded LSM database for cluster state

**Rationale**:
- Fast, embeddable (no separate service)
- Write-optimized (good for frequent state updates)
- Synchronous writes for durability
- Rust bindings available
- Proven in production systems

**Trade-off**: Single-machine database (replicated via Raft)

### LQL (SQL-Inspired Query Language)
**Decision**: Custom query language instead of JSON DSL

**Rationale**:
- Familiar to SQL users (larger audience)
- More readable than nested JSON
- Composable with pipe operator (Unix philosophy)
- Can be optimized for search operations
- Easier to parse and validate

**Trade-off**: New language to learn vs using JSON DSL

### Raft Consensus (Planned)
**Decision**: Use Raft for distributed consensus

**Rationale**:
- Well-understood algorithm (published, proven)
- Better than Paxos for implementation
- Leader-based (simpler logic)
- Good performance in normal case
- Handles partition tolerance

**Trade-off**: Still planning Phase 2 implementation

## Architectural Decisions

### Layered Architecture
**Decision**: Strict layer separation (Client → Protocol → Gateway → Coordination → Query → Index → Storage)

**Rationale**:
- Clear separation of concerns
- Testable at each layer
- Easier to replace components (e.g., Tantivy → alternative)
- Performance debugging isolation
- Scaling decisions per layer

**Trade-off**: More inter-layer communication overhead

### Shard Router as Separate Component
**Decision**: Explicit shard routing logic instead of client-side hashing

**Rationale**:
- Topology changes transparent to clients
- Rebalancing without client notification
- Replica migration without disruption
- Consistent routing across all requests
- Gateway is single point of routing truth

**Trade-off**: Shard router becomes bottleneck (mitigated by caching)

### Primary-Replica Replication
**Decision**: Single primary, N-1 replicas per shard

**Rationale**:
- Write consistency (all writes go to one node)
- Read scalability (reads can hit any replica)
- Failure recovery simpler than quorum
- Tunable consistency levels (ONE, QUORUM, ALL)

**Trade-off**: Primary failure requires promotion delay

### Segment-Based Indexing
**Decision**: Multiple searchable segments merged periodically

**Rationale**:
- Tantivy default (proven)
- Near real-time indexing possible
- Background merging doesn't block searches
- Incremental compaction
- Faster writes vs single-segment

**Trade-off**: Search costs increase with segment count

### Filesystem for Index Data, RocksDB for Metadata
**Decision**: Two-tier storage (hot data + metadata separation)

**Rationale**:
- Filesystem handles large, read-heavy index data efficiently
- RocksDB handles small, frequent metadata updates
- Each optimized for its access pattern
- Standard practice (Elasticsearch model)

**Trade-off**: Two storage systems to manage

## API Design

### OpenAPI 3.0 Compliance
**Decision**: Full OpenAPI spec generation via utoipa

**Rationale**:
- Swagger UI auto-generation
- Client code generation possible
- Standardized API documentation
- Type-safe in Rust (utoipa macro)

**Trade-off**: Added compile-time macro processing

### Streaming Responses (StreamableHTTP)
**Decision**: Server-Sent Events for large result sets

**Rationale**:
- Client can process results as they arrive
- Reduces memory on client side
- Progressive loading (better UX)
- Backpressure handling built-in

**Trade-off**: Client needs streaming support

## Security Model

### API Key + Bearer Token Support
**Decision**: Multiple auth methods instead of single standard

**Rationale**:
- API keys for client tools/integrations
- Bearer tokens for user applications
- Basic auth for simplicity
- mTLS for inter-node (planned)
- No single point of failure

**Trade-off**: More auth logic to test/maintain

### Rate Limiting per API Key
**Decision**: Token bucket algorithm for throttling

**Rationale**:
- Fair resource allocation across clients
- Prevents abuse/DOS
- Smooth over burst traffic
- Industry standard approach

**Trade-off**: Adds latency check to request path

## Testing Strategy

### Layered Testing
**Decision**: Unit tests (per module) + integration tests (across modules) + E2E (full stack)

**Rationale**:
- Catches bugs at lowest level (fast feedback)
- Integration tests catch interface mismatches
- E2E validates real workflows
- Pyramid structure (many units, few E2E)

**Trade-off**: More tests to maintain

### 278+ Tests with >95% Coverage
**Decision**: Comprehensive test suite from Phase 1

**Rationale**:
- Alpha quality requires high confidence
- Foundation must be solid
- Bug fix easier when coverage exists
- Refactoring safer with tests

**Trade-off**: Slower development initially

## Platform Compatibility

### Windows Native Path Workaround
**Decision**: Use Windows native paths, not WSL `/mnt/` mounts

**Rationale**:
- Tantivy has filesystem compatibility issues with WSL 9p protocol
- Native Windows path avoids protocol mismatch
- Simple documentation-based solution
- No code changes required

**Trade-off**: WSL users must use Linux native paths or Windows native

### Cross-Platform Build Support
**Decision**: Single Dockerfile for all platforms

**Rationale**:
- Linux base (Debian) widely supported
- CI/CD matrices for Windows/macOS testing
- Container isolates platform differences
- Standard Docker approach

**Trade-off**: Native binary development requires platform-specific setup

## Operational Decisions

### Single Configuration File (YAML)
**Decision**: `config.yml` with environment variable overrides

**Rationale**:
- Human-readable format
- Environment override for containerization
- Centralized config (not scattered)
- Schema validation possible

**Trade-off**: YAML complexity vs simplicity

### Structured JSON Logging
**Decision**: JSON logs, pretty-print in dev, machine-readable in prod

**Rationale**:
- Log aggregation ready (ELK, Loki)
- Structured fields for filtering
- Correlation IDs for tracing
- Machine-parseable

**Trade-off**: Less human-readable in raw form

### Health Check Endpoint
**Decision**: Simple `/_cluster/health` probe

**Rationale**:
- Container health checks use it
- Load balancers can probe it
- Indicates readiness without metrics overhead
- HTTP standard practice

**Trade-off**: Limited diagnostic info (quick check only)

## Known Limitations & Decisions

### WSL Tantivy Issues
**Decision**: Document and recommend Windows Native, don't migrate off Tantivy

**Rationale**:
- WSL issue, not Lexum issue
- Tantivy benefits outweigh WSL pain
- Windows native works perfectly
- Migration to SQLite FTS5 feasible but unnecessary

**Future**: Phase 2 may explore alternatives if WSL support critical

### Alpha Status Acceptable for Cortex
**Decision**: Can integrate Lexum in Cortex despite alpha status IF dependencies met

**Rationale**:
- Cortex still early (phase11)
- Lexum Phase 1 feature-complete
- Cortex can drive Lexum Phase 2 requirements
- Shared team (HiveLLM)

**Condition**: Cortex waits for Phase 2 completion or accepts stability risk
