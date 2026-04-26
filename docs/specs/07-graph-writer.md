# 07 — Graph writer (Nexus client)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 02

## Goal

Translate enriched events into the Cortex graph schema (nodes + edges from architecture §4.2) and write them to **Nexus** idempotently, in batches, with stable node identity and edge deduplication. The graph writer is the *link layer* — it is what makes retrieval by neighborhood (Cypher traversals) possible. It does not own Nexus; it is a client.

## Scope

**In:**
- Worker consuming `cortex.events.enriched` + `cortex.events.embedded`.
- Node/edge mapping from events to the graph schema.
- Nexus client (HTTP / Bolt) with batched transactions, retry, and backoff.
- Stable node identity across re-runs (content-hash or natural key).
- Edge deduplication.
- Schema bootstrapping (indexes, constraints).
- Telemetry.

**Out:**
- Cypher query surface for callers (spec 11 owns the read path).
- Graph-analytics jobs (degree stats, community detection) — future.
- Nexus operations (deployment, backups) — owned by Nexus team.
- UI/graph explorer (spec 16).

## Inputs / Outputs

### Trait

```rust
#[async_trait]
pub trait GraphWriter: Send + Sync {
    async fn write_batch(&self, events: &[EnrichedEvent]) -> Result<GraphWriteReport>;
}

pub struct GraphWriteReport {
    pub nodes_upserted: u32,
    pub edges_upserted: u32,
    pub nodes_deduped: u32,
    pub edges_deduped: u32,
    pub by_label: BTreeMap<String, u32>,
    pub latency_ms: u32,
}
```

### Schema (labels & edge types)

Mirrors architecture §4.2 exactly. Nodes:

| Label           | Natural key                                                | Required props                              |
|-----------------|------------------------------------------------------------|---------------------------------------------|
| `Session`       | `session_id`                                                | `started_at`, `adapter`, `model`, `repo?`   |
| `Turn`          | `turn_id`                                                   | `session_id`, `ts`, `role`, `kind`          |
| `ToolCall`      | `tool_call_id`                                              | `turn_id`, `tool_name`, `ts`, `status`      |
| `Artifact`      | `(repo, path, content_hash)` composite                      | `kind` (`code`/`doc`), `language?`, `bytes` |
| `Decision`      | `decision_id` (ULID)                                        | `title`, `status`, `created_at`             |
| `Memory`        | `memory_id`                                                 | `scope`, `author`, `ts`                     |
| `Analysis`      | `analysis_id`                                               | `question`, `status`, `opened_at`           |
| `Law`           | `law_id` (e.g. `LAW-007`)                                   | `title`, `severity`, `version`              |
| `LawViolation`  | `violation_id` (ULID)                                       | `law_id`, `turn_id`/`tool_call_id`, `ts`    |
| `Model`         | `(vendor, name, version)` composite                         | — pure registry                             |
| `Repo`          | `repo` (string)                                             | `origin_url?`                               |

Edges (all undirected-rendered but stored directed):

| Edge                              | From → To                       | Optional props               |
|-----------------------------------|---------------------------------|------------------------------|
| `HAS_TURN`                        | `Session → Turn`                | —                            |
| `HAS_TOOL_CALL`                   | `Turn → ToolCall`               | —                            |
| `TOUCHED`                         | `ToolCall → Artifact`           | `operation` (read/write/exec)|
| `LINKED_TO`                       | `Turn → Decision`               | `role` (proposed/applied/superseded) |
| `CITES`                           | `Turn → Decision`               | —                            |
| `REMEMBERS`                       | `Session|Turn → Memory`         | —                            |
| `DEBATED_IN`                      | `Turn → Analysis`               | `round`                      |
| `RESOLVES_TO`                     | `Analysis → Decision`           | —                            |
| `SUPERSEDES`                      | `Decision → Decision`           | `reason`                     |
| `OF`                              | `LawViolation → Law`            | —                            |
| `OBSERVED_IN`                     | `LawViolation → Turn|ToolCall`  | `evidence_json`              |
| `SIMILAR_TO`                      | `* → *`                         | `score` (derived from KNN, spec 11) |
| `IN_REPO`                         | `Artifact → Repo`               | —                            |
| `USED_MODEL`                      | `Session → Model`               | —                            |

`SIMILAR_TO` is **not** written by this worker — it is materialized on-demand by the query orchestrator (spec 11). Listed here for completeness.

### Event-to-graph mapping

Each enriched event emits a *graph patch*: a deduplicated list of `(node|edge, op)` entries. The worker computes the patch, then sends it to Nexus as a single `UNWIND` Cypher statement.

Examples:

- **`tool_call.*`** → upsert `ToolCall`, upsert `HAS_TOOL_CALL(Turn→ToolCall)`, upsert `TOUCHED(ToolCall→Artifact)` for each affected file, upsert `Artifact` and `IN_REPO`.
- **`turn.*`** → upsert `Turn`, upsert `HAS_TURN(Session→Turn)`, create `Session` if missing.
- **`decision.created`** → upsert `Decision`, add `LINKED_TO(Turn→Decision, role=proposed)`.
- **`law.violation`** → upsert `LawViolation`, upsert `OF(→Law)`, upsert `OBSERVED_IN(→Turn|ToolCall)`. `Law` node must already exist (seeded by spec 13).

Mapping lives in `cortex-workers/src/graph/mapper.rs`; one `fn map(&EnrichedEvent) -> GraphPatch` with an exhaustive match on `kind`.

## Design

### Stable identity

- **Natural keys** (e.g. `session_id`, `turn_id`) are used verbatim as the Nexus node key. Nexus `MERGE` semantics handle idempotency.
- **Composite keys** (e.g. `Artifact (repo, path, content_hash)`) are concatenated `repo|path|content_hash` and stored as `natural_key`; Nexus has a unique index on `Artifact.natural_key`.
- **Generated ULIDs** (`Decision`, `Analysis`, `LawViolation`) are produced upstream (by `cortex-core` or the adapter) and passed through — the graph writer never mints new IDs.

### Nexus client

```rust
pub struct NexusClient {
    endpoint: Url,                     // bolt:// or http://
    auth: Option<(String, String)>,
    pool: deadpool::Pool<NexusConn>,   // connection pool
    retry: RetryPolicy,                // 3 attempts, exp backoff 100/400/1600 ms
}

impl NexusClient {
    async fn run_write_tx(&self, batch: &GraphPatch) -> Result<WriteStats>;
    async fn ensure_schema(&self, statements: &[&str]) -> Result<()>;
}
```

- **Transport:** Bolt preferred (persistent connection, lower overhead); HTTP fallback via flag `CORTEX_GRAPH_TRANSPORT=http`.
- **Batching:** one Cypher transaction per 256 graph-patch entries (nodes+edges combined). Larger than embedder batches because Nexus is transactional and round-trip cost dominates.
- **Retry policy:** retry on transient network / Nexus 5xx; do **not** retry on constraint violations (hard error → dead-letter).

### Cypher generation

All writes go through a single parametrized `UNWIND` template, for example for `ToolCall`:

```cypher
UNWIND $rows AS row
MERGE (tc:ToolCall {id: row.id})
SET tc += row.props
WITH tc, row
MATCH (t:Turn {id: row.turn_id})
MERGE (t)-[r:HAS_TOOL_CALL]->(tc)
SET r.ts = coalesce(r.ts, row.ts);
```

One template per (label × incoming edge) pattern. Templates live in `cortex-workers/cypher/` as `.cypher` files loaded at startup. No string concatenation of user data.

### Schema bootstrapping

On worker startup:

```cypher
CREATE CONSTRAINT session_id IF NOT EXISTS FOR (s:Session) REQUIRE s.id IS UNIQUE;
CREATE CONSTRAINT turn_id IF NOT EXISTS FOR (t:Turn) REQUIRE t.id IS UNIQUE;
CREATE CONSTRAINT tool_call_id IF NOT EXISTS FOR (tc:ToolCall) REQUIRE tc.id IS UNIQUE;
CREATE CONSTRAINT artifact_natural_key IF NOT EXISTS FOR (a:Artifact) REQUIRE a.natural_key IS UNIQUE;
CREATE CONSTRAINT decision_id IF NOT EXISTS FOR (d:Decision) REQUIRE d.id IS UNIQUE;
CREATE CONSTRAINT law_id IF NOT EXISTS FOR (l:Law) REQUIRE l.id IS UNIQUE;
CREATE INDEX artifact_repo_path IF NOT EXISTS FOR (a:Artifact) ON (a.repo, a.path);
CREATE INDEX turn_ts IF NOT EXISTS FOR (t:Turn) ON (t.ts);
CREATE INDEX tool_call_name IF NOT EXISTS FOR (tc:ToolCall) ON (tc.tool_name);
```

Idempotent; runs every startup. Failure here is fatal — no writes happen without schema.

### Concurrency

```
Synap consumer ──▶ mapper ──▶ patch coalescer ──▶ Nexus write-tx ──▶ publisher (cortex.events.graphed)
```

**Patch coalescer:** deduplicates node/edge upserts within a micro-batch (same `TOUCHED(ToolCall, Artifact)` seen twice in one window is written once). Cuts Nexus work by ~40% on bootstrap traffic where many events touch the same file.

**Knobs (env):**
- `CORTEX_GRAPH_WORKERS=4`
- `CORTEX_GRAPH_PATCH_BATCH=256`
- `CORTEX_GRAPH_FLUSH_MS=500`
- `CORTEX_GRAPH_MAX_RETRY=3`

Backpressure: if Nexus returns `TransientError` sustained for >30 s, the worker pauses the consumer.

### Failure modes

| Failure                                   | Handling                                                                |
|-------------------------------------------|-------------------------------------------------------------------------|
| Constraint violation (e.g. duplicate ULID for Decision) | Fail the event → dead-letter (`cortex.events.invalid`); this is a bug upstream |
| Nexus `TransientError` / 5xx              | Retry with exp backoff; eventually pause consumer                       |
| Unknown event kind                        | Skip with warning; counter `graph.unknown_kind`                         |
| Missing Turn for a ToolCall (out-of-order) | Buffer up to 30 s; if still missing, fabricate an `Orphan:true` Turn node and log |
| Nexus auth failure                        | Fail fast at startup                                                    |
| Schema statement fails                    | Fail fast at startup                                                    |

### Observability

```
cortex.graph.nodes.upserted       counter, labels: label
cortex.graph.edges.upserted       counter, labels: type
cortex.graph.dedup.hits           counter, labels: kind (node|edge)
cortex.graph.tx.latency_ms        histogram
cortex.graph.tx.size              histogram (ops per transaction)
cortex.graph.errors               counter, labels: category
cortex.graph.orphans              counter, labels: parent_label
cortex.graph.backpressure.active  gauge
```

## Acceptance criteria

- [ ] Startup against empty Nexus creates all constraints + indexes; re-startup is a no-op.
- [ ] 10 000-event synthetic stream (mix of `turn`, `tool_call`, `decision`) results in correct Cypher counts: `Session` nodes = distinct session_ids; `Turn` nodes = distinct turn_ids; `HAS_TURN` edges = `Turn` count; etc.
- [ ] Idempotency: replaying the same 10 000-event stream results in zero new nodes/edges (`nodes_deduped == nodes_upserted`, same for edges).
- [ ] Patch coalescer: 100 events all touching `Artifact(vectorizer, src/lib.rs)` produce **one** Artifact upsert and **100** TOUCHED edges (edges are not deduped across events; nodes are).
- [ ] Out-of-order handling: a `tool_call` arriving before its `turn.start` is buffered ≤30 s and resolves correctly when the `turn.start` arrives; after 30 s, an orphan Turn is created and counter increments.
- [ ] Constraint violation (synthetic duplicate `Decision.id`) routes to `cortex.events.invalid`, other events in the batch succeed, transaction is split.
- [ ] Nexus 5xx storm: 429 soak for 1 minute → worker drains successfully after the storm, zero events lost.
- [ ] Schema drift: starting against a Nexus that already has an incompatible constraint fails at startup with a clear message.
- [ ] Bolt vs HTTP: both transports pass the full suite under `CORTEX_GRAPH_TRANSPORT` flag.
- [ ] Telemetry counters non-zero after soak; P95 tx latency < 300 ms on dev stack.

## Decisions

1. **MERGE-based idempotency, not lookup-then-write.** Nexus `MERGE` with unique constraints is the only thread-safe path. Never read-check-write.
2. **Natural keys where they exist; composite otherwise.** Avoid generating surrogate IDs for things that already have a natural identity (`Artifact` is the only composite case).
3. **`SIMILAR_TO` is a read-time derivation.** Storing cross-node similarities would 10× graph size and bit-rot the moment embeddings change. The query layer computes this on demand (spec 11).
4. **Cypher templates are source-controlled files, not string builds.** Security (injection) + auditability. Every template is reviewed once, used forever.
5. **Patch coalescer lives client-side.** Nexus charges per op; coalescing before the wire saves ~40% on bootstrap.
6. **Orphans are allowed.** In distributed event streams, out-of-order arrivals are normal. Orphan `Turn{orphan:true}` nodes are a debuggable signal, not a data-loss error.

## Open questions

1. **Per-repo graph partitions.** Should we partition the graph per repo to isolate blast radius of schema changes? Defer — Nexus HA story (architecture §11 Phase 5) will decide.
2. **Soft-delete semantics.** When a repo is forgotten (`cortex forget --repo X`), do we detach-delete all `Artifact` nodes, or mark them `deleted=true`? Leaning detach-delete; revisit with legal/compliance input in Phase 3.

## References

- Architecture §4.2 (graph schema), §5.2 (processing pipeline).
- Spec 01 — Event schema (source of all node/edge data).
- Spec 02 — Storage layout (Nexus instance sizing, Parquet archive as system of record).
- Spec 04 — Cortex Core (produces `EnrichedEvent`).
- Spec 06 — Embedder (sibling consumer; same input stream).
- Spec 11 — Query API (reader; `SIMILAR_TO` derivation).
- Nexus docs: `e:/HiveLLM/Nexus/README.md`, Cypher reference, Bolt protocol.
