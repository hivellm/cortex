# Nexus: Public Surface

## Transports & URL Schemes

| Scheme | Transport | Default Port | Use case |
|--------|-----------|--------------|----------|
| `nexus://host[:port]` | Binary RPC (MessagePack) | 15475 | **Default.** CLI + SDK. Fastest. |
| `http://host[:port]` | HTTP/JSON | 15474 | Browser, firewalls, diagnostics. |
| `https://host[:port]` | HTTPS/JSON | 443 | Public-internet TLS. |
| `resp3://host[:port]` | RESP3 (Redis-style) | 15476 | Debug port; SDKs reject this with clear error. |

**Transport precedence**: URL scheme > `NEXUS_SDK_TRANSPORT` env > config field > `nexus` default.

## REST API Surface

### Cypher Execution
- **POST** `/cypher` — execute Cypher query with parameters
- **POST** `/cypher/explain` — query plan
- **POST** `/cypher/profile` — execution metrics
- **GET** `/queries` — active query list
- **DELETE** `/queries/{query_id}` — terminate query

### Multi-Database
- **GET** `/databases` — list databases
- **POST** `/databases` — create database
- **DELETE** `/databases/{name}` — drop database

### External Node IDs (Phase 9 + 10)
- **POST** `/data/nodes` — create node with external ID
  - Payload: `{label, properties, external_id, conflict_policy}`
  - Returns: `{id, _id, label, properties}`
- **GET** `/data/nodes/by-external-id` — fetch node by external ID
  - Query params: `external_id`
  - Returns: node record or 404

### Admin & Auth
- **POST** `/admin/users` — create user (admin-gated)
- **POST** `/admin/keys` — create API key
- **GET** `/cluster/status` — shard layout + health (cluster mode)
- **POST** `/cluster/add_node` — register shard node (cluster admin)
- **GET** `/replication/status` — master/replica lag (replication mode)

### Monitoring
- **GET** `/prometheus` — Prometheus metrics (query counts, cache, audit)
- **GET** `/health` — liveness check

## Cypher Language Surface

### Reading Clauses
- `MATCH`, `OPTIONAL MATCH` (nodes, rels, multiple labels, directed/undirected)
- Variable-length: `*`, `*n`, `*m..n`, `+`, `?`
- `WHERE` (after MATCH/OPTIONAL MATCH/WITH)
- `WITH` (projection, aggregation, chaining)
- `UNWIND` (into CREATE, aggregation, filter)
- `RETURN` (projection, DISTINCT, AS alias, `n.*`, complex expressions)
- `ORDER BY`, `SKIP`, `LIMIT`
- `UNION`, `UNION ALL`
- `CALL { … }` (scalar + row-level subqueries)
- Pattern comprehensions, list comprehensions, map projection
- `CASE` (simple + generic)
- `EXISTS { … }` subqueries
- Named paths: `p = (a)-[*]-(b)`, `nodes(p)`, `relationships(p)`, `length(p)`

### Writing Clauses
- `CREATE` nodes/relationships (properties, multi-label)
- `MERGE` (+ ON CREATE SET / ON MATCH SET)
- `SET` property, `SET n += map`, multi-property
- `SET n:Label` / `REMOVE n:Label` (static labels)
- `SET n:$label` / `REMOVE n:$label` / `CREATE (n:$label)` (dynamic labels)
- `DELETE` / `DETACH DELETE`
- `REMOVE` property
- `FOREACH (x IN list | …)`
- `LOAD CSV [WITH HEADERS]`
- `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` / `RELEASE SAVEPOINT`

### Constraints & Schema
- `CREATE INDEX [IF NOT EXISTS]` on single property
- Composite B-tree indexes: `ON (n.p1, n.p2, …)`
- Full-text indexes via `db.index.fulltext.createNodeIndex(…)`
- `UNIQUE` constraint
- `NODE KEY` (composite uniqueness + implicit NOT NULL)
- `NOT NULL` constraint (nodes + relationships)
- Property-type: `IS :: INTEGER|FLOAT|STRING|BOOLEAN|BYTES|LIST|MAP`
- `FOR (n:L) REQUIRE (...)` DDL (Cypher 25)

### Procedures (Selection)

**Vector**: `CALL vector.knn(label, vector, k) YIELD node, score`

**Graph Algorithms** (19 GDS procedures):
- PageRank (standard, weighted, parallel)
- Betweenness, eigenvector, Dijkstra, A*, Yen's k-paths
- Louvain, label propagation, triangle count, clustering

**APOC** (~100 procedures):
- `apoc.coll.*` (union, intersection, sort, partition, flatten, …)
- `apoc.map.*` (merge, fromPairs, groupBy, submap, …)
- `apoc.text.*` (levenshtein, jaroWinkler, regex.*, base64, …)
- `apoc.date.*` (format, parse, convertFormat, diff, toYears, …)
- `apoc.schema.*` (assert, nodes, relationships, indexExists, …)
- `apoc.util.*`, `apoc.convert.*`, `apoc.number.*`, `apoc.agg.*`

**System**: `db.indexes`, `db.constraints`, `db.labels`, `dbms.procedures()`, `dbms.functions()`

## SDK Methods (All Languages)

Every SDK ships these core methods (names vary by language convention):

### Cypher Execution
- `execute_cypher(query, parameters)` → `{columns, rows, execution_time_ms}`
- `explain_cypher(query)` → query plan
- `profile_cypher(query)` → metrics

### External Node IDs (Phase 10 surface)
- `create_node_with_external_id(label, properties, external_id, conflict_policy)` → node
- `get_node_by_external_id(external_id)` → node or null

**Conflict policies**: `ERROR` | `MATCH` | `REPLACE`

### Multi-Database
- `list_databases()` → names
- `create_database(name)` → result
- `drop_database(name)` → result
- `use_database(name)` → switch context

### Admin (auth-gated)
- `create_user(username, password)` → user record
- `create_api_key(name)` → key + secret

## Binary RPC Wire Format

- **Framing**: length-prefixed MessagePack (u32 BE length + msgpack payload)
- **Port**: 15475 (default)
- **Multiplexing**: per-connection; supports pipelined requests
- **Error handling**: msgpack-encoded error frames with code + message
- **Special types**: NexusValue enum (Node, Relationship, Bytes-native embeddings, etc.)

## RESP3 Debug Port

- **Port**: 15476 (opt-in via `NEXUS_RESP3_ENABLED=true`)
- **Commands**: `HELLO 3`, `AUTH`, `CYPHER <query>`, `STATS`
- **Use case**: `redis-cli` / `iredis` compatibility for interactive debugging
- **Semantics**: Not Redis emulation; `SET key value` returns error

## CLI (`nexus` binary)

```bash
nexus query "RETURN 1 + 2"
nexus db list / create / drop / switch
nexus user create / delete / list
nexus key create / delete / list
nexus schema show / index list / constraint list
nexus data export / import
```

Default transport: `nexus://127.0.0.1:15475` (RPC). Respects `NEXUS_URL`, `NEXUS_TRANSPORT` env vars.
