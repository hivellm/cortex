# Lexum Public Surface

## REST API (39 Endpoints, 100% Working)

### Base URL
```
http://localhost:9200
```

### Cluster API
- `GET /` — Cluster info
- `GET /_cluster/health` — Health status
- `GET /_cluster/stats` — Cluster statistics
- `GET /_nodes` — Node information
- `GET /_settings` — Global settings

### Index Management
- `PUT /api/v1/indices/{index}` — Create index
- `GET /api/v1/indices/{index}` — Get index info
- `DELETE /api/v1/indices/{index}` — Delete index
- `GET /api/v1/indices` — List indices
- `PUT /api/v1/indices/{index}/settings` — Update settings
- `PUT /api/v1/indices/{index}/mappings` — Update mappings

### Document Operations
- `POST /api/v1/indices/{index}/documents` — Add document
- `GET /api/v1/indices/{index}/documents/{id}` — Get document
- `PUT /api/v1/indices/{index}/documents/{id}` — Update document
- `DELETE /api/v1/indices/{index}/documents/{id}` — Delete document
- `POST /api/v1/indices/{index}/bulk` — Bulk operations

### Search
- `POST /api/v1/indices/{index}/search` — Standard search
- `GET /api/v1/indices/{index}/_search/stream` — Streaming search
- `POST /api/v1/lql` — LQL query execution
- `GET /api/v1/search/explain` — Query explanation

### Templates & Snapshots
- `PUT /api/v1/templates/{name}` — Create template
- `GET /api/v1/templates/{name}` — Get template
- `DELETE /api/v1/templates/{name}` — Delete template
- `POST /api/v1/snapshots` — Create snapshot
- `GET /api/v1/snapshots/{id}` — Get snapshot
- `POST /api/v1/snapshots/{id}/restore` — Restore from snapshot

### Admin
- `GET /_metrics` — Prometheus metrics
- `GET /health` — Simple health check
- `GET /swagger-ui` — OpenAPI/Swagger UI

### Response Format
```json
{
  "success": true,
  "data": { /* response */ },
  "meta": { "took": 42, "timestamp": "2024-10-25T10:00:00Z" }
}
```

## CLI Tool (8 Command Groups)

### Invocation
```bash
lexum <command> <subcommand> [options]
# or
cargo run --bin lexum-cli -- <command> [args]
```

### Command Groups

1. **index** — Index management (create, delete, list, get)
2. **document** — Document operations (add, get, update, delete, bulk)
3. **search** — Query execution (lql, match, term, range)
4. **snapshot** — Backup/restore (create, list, restore, delete)
5. **template** — Index templates (create, list, delete)
6. **cluster** — Cluster operations (health, stats, settings)
7. **server** — Server control (start, stop, status)
8. **repl** — Interactive shell

### CLI Examples
```bash
lexum server start --daemon
lexum index create products schema.yml
lexum doc bulk products products.json
lexum lql products "FROM products WHERE price:[100,500]"
lexum snapshot create backup snap_2024 --indices products --wait
lexum repl
```

## LQL Query Language

### Syntax
```sql
FROM <index> [| <operation>]*
```

### Operations (9 Query Types)
- **FROM**: Source index selection
- **WHERE**: Filtering with boolean logic
- **MATCH**: Full-text search with scoring
- **SELECT**: Project specific fields
- **GROUP BY**: Aggregation grouping
- **SORT**: Result ordering
- **LIMIT**: Result pagination
- **JOIN**: Index joins (planned)
- **AGGREGATE**: Complex aggregations

### Example
```sql
FROM products
| WHERE category = "electronics" AND price > 100
| MATCH "gaming laptop" IN title
| GROUP BY brand
| SORT score DESC
| LIMIT 50
```

## Authentication Methods

1. **API Key Header**: `X-API-Key: your-api-key`
2. **Bearer Token**: `Authorization: Bearer your-token`
3. **Basic Auth**: `-u username:password`
4. **mTLS**: Client certificates for inter-node communication

## Protocol Support

- **HTTP/2**: Primary protocol with streaming support
- **MCP**: Model Context Protocol (search, retrieve, aggregate, stream operations)
- **UMICP**: Binary protocol with compression and multiplexing (planned)
- **WebSocket**: Real-time subscriptions (planned)
