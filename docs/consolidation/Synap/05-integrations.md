# Synap — Integrations

## HiveLLM Project Integration

### Cortex
- **Relationship**: Synap is a consumable data service for Cortex
- **Usage**: Cortex ingest pipeline may use Synap queues for task distribution
- **Integration Point**: Cortex reads from Synap streams/queues via Rust SDK

### Nexus
- **Relationship**: Nexus (knowledge graph DB) is separate; Synap complements it
- **Usage**: Synap can cache Nexus queries or relay Nexus events
- **No Direct Coupling**: Both are independent HiveLLM services

### Vectorizer
- **Relationship**: Vectorizer (embedding service) is independent
- **Usage**: Synap can queue embedding jobs for Vectorizer
- **Integration**: Via HTTP or SDK to Vectorizer; results stored in Synap

### Expert
- **Relationship**: Expert (decision engine) independent
- **Usage**: Synap can queue decisions, stream Expert outputs
- **Integration**: SDK-based inter-service messaging

### Lexum
- **Relationship**: Full-text search service (alternative to Meilisearch)
- **Usage**: Synap acts as cache layer or event broker for Lexum results
- **No Direct Dependency**: Cortex uses Meilisearch (not Lexum) as of now

### Rulebook
- **Relationship**: Task/project management framework
- **Usage**: Synap stores queue for background task execution
- **Integration**: via SDK or HTTP for task dispatch

## Protocol Integrations

### MCP (Model Context Protocol)
- Status: Production ready
- Features: Configurable tools for KV, Hash, List, Set, Queue, Sorted Set
- Endpoint: `/mcp`
- Security: Full authentication support

### UMICP (Universal Matrix Inter-Communication Protocol)
- Status: Production ready
- Features: 13 operations via MCP bridge, TLS support
- Use Case: Matrix protocol interop

## SDK Usage

All HiveLLM services use official Synap SDKs (Rust, Python, TypeScript) for type-safe access:
```rust
let client = SynapClient::new("synap://synap-server:15501").await?;
client.set("key", "value").await?;
```
