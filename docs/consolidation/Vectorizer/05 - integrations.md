# Vectorizer — Integrations within HiveLLM

**Last Updated:** 2026-05-04

## Consumed By

### Cortex (Primary Consumer)

**Vector Lane** (`crates/cortex-api/src/vector_lane.rs`):
- Semantic search for decision context (pre_change_context, similar_problems, free_search)
- Hybrid search combining sparse (BM25) + dense (semantic) retrieval
- Graph traversal for related decisions and multi-hop relationships
- Per-key rate limiting + audit logging

**Auth Flow:**
- Resolves credentials from `CORTEX_VECTORIZER_API_KEY` or `CORTEX_VECTORIZER_USER + PASSWORD`
- Caches JWT locally, reactively refreshes on 401
- Optional periodic JWT warmup (env: `CORTEX_VECTORIZER_JWT_WARMUP_SECS`)
- Falls back to `MemoryVectorLane` on persistent auth failure

**Integration Points:**
- `cortex-api` boots probe: `list_collections()` to verify authenticated connection
- Query routing: decision context uses `search_hybrid` (dense + RRF)
- Multi-collection search: related decisions via `multi_collection_search()`
- Graph enrichment: edges auto-discovered from decision metadata

See also: `docs/operations/vectorizer-auth.md` (Cortex repo)

### CompressionPrompt

**Context Compression via Vectorizer Embeddings:**
- Stores compressed prompts in a Vectorizer collection (sparse + dense)
- Uses `search_intelligent()` (query expansion) to retrieve similar past compressions
- Applies BM25 ranking to select top compressions by semantic relevance
- Reduces LLM context window by semantic clustering

**Crate Location:** (CompressionPrompt repo, not Vectorizer)

**Calls Made:**
- `create_collection()` — bootstrap compression index
- `insert_text()` — store compressed prompts with provenance metadata
- `search_hybrid()` — retrieve similar compressions (BM25 + dense re-ranking)

### Nexus (Graph Database)

**Vector Enrichment & External ID Correlation:**
- Vectorizer provides semantic embedding surface for Nexus node metadata
- Hybrid search over Nexus external-ID fields (BM25 + dense)
- Graph edges between vectors and Nexus nodes (via `graph_create_edge()`)
- Distributed sharding (Vectorizer can shard; Nexus queries shard-aware)

**Crate Location:** (Nexus repo, consumes via SDK)

**Calls Made:**
- Search across Nexus collections enriched with vector payloads
- Graph relationship discovery (external IDs → vector IDs)
- Multi-collection search over Nexus-federated collections

**phase11l Note:** Nexus v2.1.0 shipped `create_node_with_external_id`, `get_node_by_external_id`, `conflict_policy ∈ {error, match, replace}`. Vectorizer SDK parity: Cortex pins `nexus-graph-sdk = "2.1"` as of phase11l §1 completion.

### Synap (Prompt Synthesis)

**Embedding Availability & Selection:**
- Vectorizer reports supported embedding models (TF-IDF, BM25, BERT, MiniLM, custom)
- Synap queries `/collections/{name}` to inspect embedding_type
- Selects embedding at insertion time based on Synap's semantic intent
- Reuses Vectorizer's quantized models to avoid re-embedding

**Crate Location:** (Synap repo, consumes via SDK)

**Calls Made:**
- `list_collections()` — discover available embeddings
- `get_collection_info()` — inspect embedding_type for selection
- `insert_text()` — store synthesized prompts with embedding
- `search()` — retrieve similar synthesized prompts by embedding

## Provides To

### AI IDEs & Agents

**MCP Server (31 tools):**
- Cursor, Claude Desktop, and other MCP-aware IDEs
- Tools for semantic search, document processing, graph traversal, file discovery
- Configuration via `claude.json` or `cursor.json`:
  ```json
  {
    "mcpServers": {
      "vectorizer": {
        "url": "http://localhost:15002/mcp",
        "type": "streamablehttp"
      }
    }
  }
  ```

**REST API:**
- Direct HTTP clients (custom integrations, scripts)
- Dashboard at `http://localhost:15002/dashboard/`

**gRPC (Qdrant-Compatible):**
- Polyglot services speaking Qdrant wire format
- Full API parity for search, insert, delete, collections

## Service Dependencies

### Upstreams (Vectorizer depends on)

**None at runtime.** Vectorizer is self-contained:
- HNSW, quantization, embedding models all in-process
- Embedding inference runs locally (TensorFlow / Candle CPU)
- No external vector store dependencies (unlike Qdrant, which may use Redis)

**At build time:**
- Rust toolchain (1.92+)
- C++ compiler for Candle / FastEmbed models (optional, feature-gated)

### Downstreams (Services depending on Vectorizer)

- **Cortex** (tightly coupled, required for decision context)
- **CompressionPrompt** (optional, improves compression quality)
- **Nexus** (optional, enriches graph relationships)
- **Synap** (optional, impacts prompt selection)
- **CLI tools** (optional, for ops)
- **AI IDEs** (optional, via MCP)

## Network Topology

### Deployment (Typical)

```
┌─────────────────┐
│  Cortex API     │ (queries vector lane)
├─────────────────┤
│ CompressionPrompt
├─────────────────┤
│  Nexus          │ (queries graph edges)
├─────────────────┤
│  Synap          │ (selects embeddings)
└────────┬────────┘
         │ HTTP / RPC / gRPC
         ↓
┌─────────────────────────────────┐
│  Vectorizer (isolated container) │
│  Port 15002 (HTTP) + 15503 (RPC) │
└─────────────────────────────────┘
```

### Docker Compose Integration

**Services:**
- `vectorizer` (hivehub/vectorizer:latest)
- `cortex-api` (depends_on: vectorizer)
- `cortex-embedder` (depends_on: vectorizer)
- `nexus` (depends_on: vectorizer, optional)
- `synap` (depends_on: vectorizer, optional)

**Network:** Shared Docker network (bridge or user-defined)

**Environment:**
- Vectorizer auth: `VECTORIZER_AUTH_ENABLED=true`
- Cortex creds: `CORTEX_VECTORIZER_USER`, `CORTEX_VECTORIZER_PASSWORD`
- Volume mounts: `vectorizer-data:/vectorizer/data` (persistent)

### Auth & Isolation

**Per-Service API Keys:**
- Cortex: `CORTEX_VECTORIZER_API_KEY` (scoped to cortex-specific collections)
- CompressionPrompt: separate key with compression-only scopes
- Nexus: separate key with graph-aware scopes
- Synap: read-only key for embedding discovery

**Collection Scoping:**
- Each service has dedicated collections (e.g., `cortex-decisions`, `compression-index`, `nexus-graph`)
- Scoped API keys restrict cross-service access (API key scope validation)
- HiveHub cluster mode enforces tenant isolation (cortex, compression-prompt, nexus, synap = isolated tenants)

## Coordination Points

### Shared Collections

When multiple services reference the same collection:
1. Only one service writes (writer holds lock)
2. Readers use `.vecdb` snapshots (WAL-driven updates)
3. Graph layer shared via read-only `_graph.json` files
4. Conflict resolution via `conflict_policy` (if integrated with Nexus)

### Data Flow

```
Cortex Decision Engine
         │ (requests semantic search)
         ↓
Cortex Vector Lane
         │ (queries)
         ↓
Vectorizer (semantic search + RRF hybrid)
         │ (returns ranked results)
         ↓
Decision Context Enrichment
         │ (metadata from CompressionPrompt + Nexus graphs)
         ↓
LLM Context Window
```

### Example: Multi-Service Decision Lookup

1. **Cortex** receives decision query
2. **Cortex Vector Lane** calls `search_hybrid("cortex-decisions", query, 10)`
3. **Vectorizer** ranks via BM25 + HNSW (< 3ms)
4. **Cortex** retrieves top-3 results with file paths
5. **Nexus** optionally queries external IDs for graph enrichment
6. **CompressionPrompt** optionally fetches similar compressions via `search_intelligent()`
7. **Synap** optionally checks embedding model compatibility
8. **Cortex** assembles final decision context → LLM

All services use same `vectorizer-sdk` for consistency.

## versioning & Compatibility

**Server-SDK coupling:**
- Server v3.3.0 ↔ Rust SDK v3.3.0 (tight coupling)
- Server v3.3.0 ↔ TypeScript/Python/Go/C# SDK v3.0.x (REST API compatible)

**Breaking changes:**
- v3.0 → v3.1: API key usage metrics + permissions update (additive)
- v3.1 → v3.2: Cluster failover + tier demotion (additive)
- v3.2 → v3.3: Hardened auth + CSRF (additive, opt-in dev mode)

**Migration path:** All services can upgrade independently:
1. Upgrade Vectorizer server first
2. Upgrade dependent SDKs (Cortex, CompressionPrompt, etc.) at own pace
3. REST API ensures backward compatibility

Cortex-specific: See `docs/operations/vectorizer-auth.md` for credential resolution.
