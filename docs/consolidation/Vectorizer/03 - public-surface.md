# Vectorizer — Public Surface (APIs, SDKs, CLI)

**Last Updated:** 2026-05-04

## HTTP REST API

**Base URL:** `http://localhost:15002` (or `https://` with TLS)

### Authentication

All endpoints require `Authorization` header:
- **JWT:** `Authorization: Bearer <token>` (issued via `POST /auth/login`)
- **API Key:** `Authorization: <key>` (long-lived, scoped)

Anonymous mode (no auth) boots with warnings; authenticated search returns 401.

### Collections

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/collections` | List all collections |
| POST | `/collections` | Create collection (POST body: name, dimension, embedding_type) |
| GET | `/collections/{name}` | Get collection metadata |
| DELETE | `/collections/{name}` | Delete collection |

### Vectors

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/vectors/{collection}/insert` | Insert vectors (batch or single) |
| GET | `/vectors/{collection}/{id}` | Get vector by ID |
| PATCH | `/vectors/{collection}/{id}` | Update vector |
| DELETE | `/vectors/{collection}/{id}` | Delete vector |
| POST | `/vectors/{collection}/search` | Semantic search (with filter + top-k) |
| POST | `/vectors/{collection}/search/hybrid` | Dense + sparse hybrid (RRF) |

### Graph Operations

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/graph/enable/{collection}` | Enable graph for collection |
| GET | `/graph/{collection}/nodes` | List nodes |
| POST | `/graph/{collection}/edges` | Create edge |
| DELETE | `/graph/{collection}/edges/{from}/{to}` | Delete edge |
| GET | `/graph/{collection}/neighbors/{node_id}` | Get neighbors |
| POST | `/graph/{collection}/path/{from}/{to}` | Shortest path |
| POST | `/graph/{collection}/discover` | Auto-discover edges |

### Authentication & Keys

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/auth/login` | Mint JWT (POST: username, password) |
| POST | `/auth/refresh` | Refresh JWT token |
| POST | `/auth/logout` | Revoke session |
| POST | `/auth/validate-password` | Validate user password |
| POST | `/auth/keys` | Create API key (optional scopes) |
| GET | `/auth/keys` | List API keys (with usage stats) |
| GET | `/auth/keys/{id}/usage` | Get key usage (per-day ring buffer) |
| PUT | `/auth/keys/{id}/permissions` | Update key scopes without rotation |
| POST | `/auth/keys/{id}/rotate` | Atomic key rotation (300s grace) |
| POST | `/auth/introspect` | RFC 7662 token introspection |
| GET | `/auth/audit` | Admin audit log (filterable) |

### Cluster & Admin

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check (no auth) |
| GET | `/metrics` | Prometheus metrics |
| POST | `/cluster/failover` | Promote replica (with WAL-lag check) |
| POST | `/cluster/replicas/{id}/resync` | Force full resync |
| POST | `/cluster/peers` | Add peer/observer |
| POST | `/cluster/rebalance` | Async shard rebalance |
| GET | `/cluster/rebalance/status` | Rebalance status |

### Special Endpoints

- `GET /dashboard/` — React SPA (embedded, ~26MB)
- `GET /graphql` — GraphQL introspection + playground
- `POST /graphql` — GraphQL queries
- `GET /mcp` — MCP protocol endpoint (StreamableHTTP)
- `GET /umicp/discover` — UMICP tool discovery

## VectorizerRPC (Binary Protocol)

**Default Port:** 15503/tcp  
**Framing:** u32 length (LE) + MessagePack body (max 64 MiB)

```rust
Request {
  id: u32,             // Client-chosen monotonic ID
  command: String,     // "dotted.command.name"
  args: Vec<Value>,    // Positional arguments (MessagePack)
}

Response {
  id: u32,                       // Echoes Request.id
  result: Result<Value, String>, // Ok(payload) or Err(message)
}
```

**Multiplexing:** Responses may arrive out-of-order. Clients match by `id`.

**Commands:** Same as REST endpoints (see vectorizer-protocol crate for wire schema).

## gRPC (Qdrant-Compatible)

Tonic-generated stubs with Qdrant API parity (see `docs/api/CLUSTER.md`):
- Search, insert, delete, collections
- Payload filtering + search groups
- Snapshots, sharding, cluster management

**Port:** 15002 (same as REST, multiplexed via HTTP/2 ALPN).

## GraphQL

**Endpoint:** `POST /graphql` (or `GET /graphql` for introspection)

**Playground:** `http://localhost:15002/graphql` (GraphiQL)

Full REST API parity via GraphQL types (collections, search, filters, mutations).

## MCP (Model Context Protocol)

**Endpoint:** `http://localhost:15002/mcp` (StreamableHTTP)

**Protocol:** JSON-RPC 2.0 over HTTP/1.1 or HTTP/2

**31 Registered Tools:**

**Core (9):**
- `list_collections`, `create_collection`, `get_collection_info`
- `insert_text`, `get_vector`, `update_vector`, `delete_vector`
- `search`, `multi_collection_search`

**Advanced Search (4):**
- `search_intelligent` (query expansion)
- `search_semantic` (cross-encoder reranking)
- `search_extra` (combined dense + sparse)
- `search_hybrid` (RRF)

**Discovery (7):**
- `filter_collections`
- `expand_queries`
- `get_file_content`, `list_files`, `get_file_chunks`
- `get_project_outline`, `get_related_files`

**Graph (8):**
- `graph_list_nodes`, `graph_get_neighbors`
- `graph_find_related`, `graph_find_path`
- `graph_create_edge`, `graph_delete_edge`
- `graph_discover_edges`, `graph_discover_status`

**Maintenance (3):**
- `list_empty_collections`, `cleanup_empty_collections`
- `get_collection_stats`

## Client SDKs

All SDKs support both `vectorizer://host[:port]` (RPC, default 15503) and `http(s)://host[:port]` (REST) URLs.

### Rust SDK (`vectorizer-sdk`)

```rust
use vectorizer_sdk::VectorizerClient;

let client = VectorizerClient::new("vectorizer://localhost:15503")?;
let results = client.search("my_collection", "query", 10).await?;
```

**Install:** `cargo add vectorizer-sdk@3.3`  
**Version:** 3.3.0 (tracks server)

### TypeScript / JavaScript SDK (`@hivehub/vectorizer-sdk`)

```typescript
import { VectorizerClient } from '@hivehub/vectorizer-sdk';

const client = new VectorizerClient('vectorizer://localhost:15503');
const results = await client.search('my_collection', 'query', 10);
```

**Install:** `npm install @hivehub/vectorizer-sdk@3.0`  
**Version:** 3.0.x (CJS + ESM compiled)

### Python SDK (`vectorizer-sdk`)

```python
from vectorizer import VectorizerClient

client = VectorizerClient('vectorizer://localhost:15503')
results = client.search('my_collection', 'query', 10)
```

**Install:** `pip install vectorizer-sdk==3.0.*`  
**Version:** 3.0.x

### Go SDK (`github.com/hivellm/vectorizer-sdk-go`)

```go
import vc "github.com/hivellm/vectorizer-sdk-go"

client := vc.NewClient("vectorizer://localhost:15503")
results, err := client.Search(ctx, "my_collection", "query", 10)
```

**Install:** `go get github.com/hivellm/vectorizer-sdk-go@v3.0`  
**Version:** 3.0.x

### C# SDK (`Vectorizer.Sdk` + `Vectorizer.Sdk.Rpc`)

```csharp
using HiveLLM.Vectorizer;

var client = new VectorizerClient("vectorizer://localhost:15503");
var results = await client.SearchAsync("my_collection", "query", 10);
```

**Install:** `dotnet add package Vectorizer.Sdk` (REST) or `Vectorizer.Sdk.Rpc` (RPC)  
**Version:** 3.0.x

## CLI Tools

**Binary:** `vectorizer` (available after `cargo install` or Docker)

| Command | Purpose |
|---------|---------|
| `vectorizer collection list` | List collections |
| `vectorizer collection create` | Create collection |
| `vectorizer vector insert` | Insert vectors (from file) |
| `vectorizer search` | Query collection |
| `vectorizer auth login` | Get JWT |
| `vectorizer api-keys create` | Create API key |
| `vectorizer backup create` | Full collection backup |
| `vectorizer migrate` | Qdrant → Vectorizer migration |

**Help:** `vectorizer --help` or `vectorizer <command> --help`

## Configuration & Environment

**Auth via env vars (precedence order):**
1. `CORTEX_VECTORIZER_API_KEY` (or `VECTORIZER_API_KEY`) — bearer token
2. `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` — login + JWT cache
3. `CORTEX_EMBEDDER_VECTORIZER_USER` + `_PASSWORD` — alias for (2)
4. *(none)* — anonymous mode with warnings

**Data directory:**
- Linux: `~/.local/share/vectorizer/`
- macOS: `~/Library/Application Support/vectorizer/`
- Windows: `%APPDATA%\vectorizer\`
- Override: `VECTORIZER_DATA_DIR` env var

**Layered config:**
```bash
VECTORIZER_MODE=production ./vectorizer
# Merges config/modes/production.yml over config/config.yml
```
