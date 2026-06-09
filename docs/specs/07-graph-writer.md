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
| `Symbol`        | `(repo, language, qualified_name)` composite                | `name`, `language`, `repo`, `qualified_name` |

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
| `DEFINES`                         | `Symbol → Artifact`             | —                            |

`SIMILAR_TO` is **not** written by this worker — it is materialized on-demand by the query orchestrator (spec 11). Listed here for completeness.

### Event-to-graph mapping

Each enriched event emits a *graph patch*: a deduplicated list of `(node|edge, op)` entries. The worker computes the patch, then sends it to Nexus as a single `UNWIND` Cypher statement.

Examples:

- **`tool_call.*`** → upsert `ToolCall`, upsert `HAS_TOOL_CALL(Turn→ToolCall)`, upsert `TOUCHED(ToolCall→Artifact)` for each affected file, upsert `Artifact` and `IN_REPO`.
- **`turn.*`** → upsert `Turn`, upsert `HAS_TURN(Session→Turn)`, create `Session` if missing.
- **`decision.created`** → upsert `Decision`, add `LINKED_TO(Turn→Decision, role=proposed)`.
- **`law.violation`** → upsert `LawViolation`, upsert `OF(→Law)`, upsert `OBSERVED_IN(→Turn|ToolCall)`. The target label of `OBSERVED_IN` is picked from the payload's `observed_event_kind` discriminator (`turn` | `tool_call`); the spec-04 schema's `allOf/if-then` enforces that the discriminator is set whenever `observed_event_id` is set, so the writer never has to guess (no phantom-node risk via MERGE). `Law` node must already exist (seeded by spec 13).
- **`artifact.*`** (`source = "code"`) → upsert `Artifact` + `IN_REPO` as before, then run the embedder's `CodeChunker` against the artifact body to extract top-level declarations and emit one `(:Symbol)-[:DEFINES]->(:Artifact)` pair per recognised symbol (phase4c §2). Artifacts whose path is outside the chunker's grammar set, or whose body has no declarations, stay Artifact-only without error.

Mapping lives in `cortex-workers/src/graph/mapper.rs`; one `fn map(&EnrichedEvent) -> GraphPatch` with an exhaustive match on `kind`.

#### Symbols & DEFINES (phase4c)

`Symbol` nodes mirror the symbol field that `cortex-embedder::CodeChunker` already stamps on every code chunk; without this layer the graph could only answer "which artifacts belong to which repo" (`IN_REPO`), not "where is `PreThinkingTool` defined?". The mapper reuses the same chunker the vector lane runs against, so the symbol set on disk in Nexus stays in lockstep with what Vectorizer indexes.

- **Natural key:** `(repo, language, qualified_name)` joined with `|`, mirroring `Artifact.natural_key`. When the chunker emits a bare name (most languages don't carry an FQN at the top-level declaration), the mapper folds the artifact path into the qualified name (`<path>::<name>`) so two `parse()` functions in different files hash to distinct Symbols. `Symbol.natural_key UNIQUE` is enforced by schema bootstrap.
- **DEFINES edge:** `(:Symbol)-[:DEFINES]->(:Artifact)`, MERGE-idempotent on the Symbol natural key plus the Artifact natural key — replay does not duplicate edges (verified by `artifact_replay_is_idempotent_under_natural_key` in `crates/cortex-graph/tests/mapper.rs`).
- **Out of scope (deferred):** `IMPORTS`, `CALLS`, `EXTENDS`, `IMPLEMENTS`. Those need richer parser-level analysis the chunker doesn't expose today; they ship in a follow-up task once the chunker emits import/call edges per chunk.

#### Semantic-edge projection (phase15b §3)

The mapper above emits the **structural / identity** edges (`HAS_TURN`, `HAS_TOOL_CALL`, `TOUCHED`, `IN_REPO`, `REMEMBERS`, `DEFINES`, …). Phase15b adds a parallel **semantic-edge projection** pass that closes the "graph lane returns nothing useful" gap by emitting twelve relationship kinds the mapper never produced:

| Edge            | From → To                     | Source                                  |
|-----------------|-------------------------------|-----------------------------------------|
| `CALLS`         | `ToolCall → <entity>`         | classifier relations (case-insensitive) |
| `IMPORTS`       | `ToolCall|Artifact → <entity>`| classifier relations                    |
| `DEFINES`       | `ToolCall|Artifact → <entity>`| classifier relations                    |
| `RETURNS`       | `ToolCall → <entity>`         | classifier relations                    |
| `SUPERSEDES`    | `Decision → Decision`         | `DecisionPayload.supersedes` (payload)  |
| `CONTRADICTS`   | `Evidence → Evidence`         | `TopicCardPayload.contradictions` (Open only) |
| `EMITTED_BY`    | `<event> → Tool|Model`        | payload producer field                  |
| `ABOUT`         | `Turn|Decision → Topic`       | `classifier.topics`                     |
| `ANSWERED_BY`   | `Turn → ToolCall`             | `Turn.tool_call_event_ids` (payload)    |
| `CITES`         | `Turn|Analysis → Decision`    | classifier relations + `DEC-\d{3,}` body regex |
| `MENTIONS_FILE` | `Turn → Artifact`             | `classifier.entities` (artifact)        |
| `RELATES_TO`    | `Turn → Turn`                 | classifier relations (cosine pass dormant — `EnrichedEvent` lacks the embedding vector) |

**Pipeline.** `cortex-workers/src/graph/projection.rs::project_envelope(env, ctx)` iterates a fixed `&[(name, fn)]` dispatch table (one row per extractor in `graph/extractors/`) and folds every emitted edge into a single edge-only `GraphPatch`. Each extractor is a pure function over `(env, ctx)`; every edge is stamped with the provenance triple (`source_event_id`, `analyzer_version`, `created_at_ms`) via `stamp_provenance` so the stale-edge sweeper can retire a superseded run.

**Worker wiring.** `graph/worker.rs` runs the projection as batch step **3b** — after the mapper's structural patch (step 3) and the static-analyzer pass (step 3a), before `write_patches`. One `ExtractCtx::new(PROJECTION_ANALYZER_VERSION)` per batch pins a single `now_ms`. The projection patch carries **edges only**: endpoint nodes are created either by the same batch's mapper patch or by a prior event's, and the writer's `MATCH (a),(b) MERGE (a)-[r]->(b)` render silently drops any edge whose endpoints are absent (tracked under `edges_dropped{edge_type}`). The two patch streams compose through the existing coalescer.

**Idempotency.** Re-projecting the same envelope is byte-identical (pinned by `project_envelope_is_idempotent_byte_identical_on_second_call`) and the `(from, to, kind)` unique constraint makes replay a no-op downstream.

**Coverage doctor.** `cortex-ops doctor-graph-coverage [--floor <share>] [--json]` queries `MATCH ()-[r]->() WHERE type(r) IN [<12 kinds>] RETURN type(r), count(r)`, renders a per-kind count + share table, and exits `1` if any kind has zero edges or falls below `--floor` (default `0.01` = 1%), `2` if Nexus is unreachable. `cortex-ops graph backfill --since <RFC3339>` re-projects archived envelopes (count-only dry-run today; payload-driven kinds produce useful counts without a classifier replay).

Resulting Cypher pattern:

```cypher
MATCH (s:Symbol {name: "PreThinkingTool"})-[:DEFINES]->(a:Artifact)
RETURN a.repo, a.path, s.language
```

## Design

### Stable identity

**Phase11l update — Nexus 2.1 reserved `_id` slot.** Cortex graph nodes carry their identity in the Nexus 2.1 reserved `_id` property. The pre-phase11l `natural_key` convention (and its per-label `IS UNIQUE` constraints) was retired in favour of Nexus's `ExternalIdIndex`. ADR-004 captures the rationale + reassessment trigger.

- **Adapter-facing identity** (`session_id`, `turn_id`, `decision_id`, …) is stamped as the canonical key string AND as the node's `_id` slot. Two surfaces, one value: SDK callers query by the semantic property (e.g. `Session.id`) while the writer's upsert path keys on `_id`.
- **Composite keys** (`Artifact (repo, path, content_hash)`, `Symbol (repo, language, qualified_name)`) are concatenated `|`-delimited and stamped as the `_id` slot. The legacy `natural_key` property still ships on the node for one soft-fallback release.
- **Generated ULIDs** (`Decision`, `Analysis`, `LawViolation`, `Memory`, `Knowledge`, `Learning`, `Consolidation`) are produced upstream (by `cortex-core` or the adapter) and passed through — the graph writer never mints new IDs.
- **Pending sentinels** (`pending|repo|path`, phase11l §5.1) cover the case where a cross-artifact `(repo, path)` lands before its canonical content_hash arrives; the §5.3 stale-edge sweeper redirects every sentinel-pointed edge once the canonical artifact is written.

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

All writes go through a single parametrized `UNWIND` template. Phase11l §3 rewrote every node template from the legacy `MERGE { natural_key }` shape to Nexus 2.1's `CREATE { _id } ON CONFLICT MATCH`:

```cypher
UNWIND $rows AS row
CREATE (n:ToolCall {_id: row.key}) ON CONFLICT MATCH
SET n += row.props
```

Edge templates kept their existing `MATCH … MERGE` shape — they match endpoints on the endpoint's identity property and benefit transparently from the index seek when that property is `_id`. One template per (label × incoming edge) pattern. Templates live in `cortex-workers/cypher/` as `.cypher` files loaded at startup. No string concatenation of user data.

Phase11k §6.1 added three read-side query templates the pre-thinking renderer loads through the same registry: `code_callers.cypher` (`:CALLS` 1-hop), `doc_trail.cypher` (`:CITES` chain), `blast_radius.cypher` (`:IMPORTS_FILE*1..2`).

### Schema bootstrapping

On worker startup (post-phase11l §4):

```cypher
-- Adapter-facing identity constraints. These properties carry
-- semantic meaning beyond the `_id` slot; the constraints are
-- belt-and-braces against an SDK regression.
CREATE CONSTRAINT session_id IF NOT EXISTS FOR (s:Session) REQUIRE s.id IS UNIQUE;
CREATE CONSTRAINT turn_id IF NOT EXISTS FOR (t:Turn) REQUIRE t.id IS UNIQUE;
CREATE CONSTRAINT tool_call_id IF NOT EXISTS FOR (tc:ToolCall) REQUIRE tc.id IS UNIQUE;
CREATE CONSTRAINT decision_id IF NOT EXISTS FOR (d:Decision) REQUIRE d.id IS UNIQUE;
CREATE CONSTRAINT memory_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.id IS UNIQUE;
CREATE CONSTRAINT analysis_id IF NOT EXISTS FOR (a:Analysis) REQUIRE a.id IS UNIQUE;
CREATE CONSTRAINT law_id IF NOT EXISTS FOR (l:Law) REQUIRE l.id IS UNIQUE;
CREATE CONSTRAINT violation_id IF NOT EXISTS FOR (v:LawViolation) REQUIRE v.id IS UNIQUE;
CREATE CONSTRAINT repo_name IF NOT EXISTS FOR (r:Repo) REQUIRE r.name IS UNIQUE;
CREATE CONSTRAINT spec_path IF NOT EXISTS FOR (s:Spec) REQUIRE s.path IS UNIQUE;

-- Secondary read-path indexes.
CREATE INDEX artifact_repo_path IF NOT EXISTS FOR (a:Artifact) ON (a.repo, a.path);
CREATE INDEX turn_ts IF NOT EXISTS FOR (t:Turn) ON (t.ts);
CREATE INDEX tool_call_name IF NOT EXISTS FOR (tc:ToolCall) ON (tc.tool_name);
CREATE INDEX symbol_repo_name IF NOT EXISTS FOR (s:Symbol) ON (s.repo, s.name);
```

Idempotent; runs every startup. Failure here is fatal — no writes happen without schema. The five `*_natural_key` uniqueness constraints shipped in phase4c-phase11k were retired in phase11l §4.1; Nexus 2.1's `ExternalIdIndex` enforces uniqueness on `_id` structurally, removing the need for per-label constraints on the legacy `natural_key` property. ADR-004 records the supersession.

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

## Read path

The write path above is consumed at query time by `cortex-api`'s
graph lane (spec 11 §Lane traits). The lane translates the
orchestrator's `GraphRequest { template, params, max_hops, scope }`
into a parametrised read-only Cypher against the same Nexus
instance the dashboard graph view talks to.

### Live lane: `NexusGraphLane`

Lives at [crates/cortex-api/src/nexus_graph_lane.rs](../../crates/cortex-api/src/nexus_graph_lane.rs).
Wired at daemon startup whenever the same `Arc<NexusClient>`
`DashboardState` already holds is non-`None` — single TCP session,
two consumers. The 2026-04-27 audit caught the asymmetry:
`cortex-api/src/main.rs` built the client for the dashboard but
the orchestrator wired `Arc::new(MemoryGraphLane::new())` — an
empty test double — so `/v1/query`'s graph lane never reached
Nexus and `results.graph_neighbors` returned empty across every
probed query.

When the client is `None` (env unset, probe failed) the lane
falls back to `MemoryGraphLane` so cold-stack dev keeps working.

### Template whitelist

The lane only executes pre-registered Cypher templates. Unknown
templates return `LaneError::Rejected` at the lane boundary —
arbitrary client-supplied Cypher never reaches Nexus.

| Template name                      | Used by intent       | Pattern                                                    |
|------------------------------------|----------------------|------------------------------------------------------------|
| `edge_artifact_touched_neighbours` | `pre_change_context` | `(:Artifact)<-[:TOUCHED]-(s)` filtered by query CONTAINS   |
| `decision_supersedes_chain`        | `decision_lookup`    | `(:Decision)-[:SUPERSEDES]->(:Decision)` chain             |
| `turn_analysis_decision_chain`     | `similar_problems`   | `(:Turn)-[:OBSERVED_IN]->(:Analysis)` (+ optional Decision)|
| `law_violations_last_30d`          | `law_check`          | `(:LawViolation)-[:VIOLATES]->(:Law)` filtered by query    |

Adding a new strategy that emits an unregistered template silently
disables graph lookups for that intent — a unit test walks the
whitelist to catch the drift early.

### Cypher shape

Every template binds `$q` from `req.params["query"]`. The Nexus
1.15 dialect ignores numeric `$param` for `LIMIT` and comparison
clauses, so each template caps at `LIMIT 50` inline (same workaround
the dashboard graph endpoint uses — see `dashboard.rs` query_nexus_graph).
The orchestrator's `req.limit` trims the result set further during
overlay derivation.

### Hit projection (Nexus row → `LaneHit`)

Each row returns 5 cells: `[edge_from, edge_to, edge_type, hops, label]`.
Projected into `LaneHit`:

| LaneHit field         | Source cell / value                                                |
|-----------------------|--------------------------------------------------------------------|
| `doc_id`              | `graph|{template}|{edge_from}|{edge_to}` (per-template namespace)  |
| `text`                | `label` (cell 4) — node title or id                                |
| `symbol`              | `edge_type` (cell 2)                                               |
| `score`               | `1.0 / max(hops, 1)` — closer hops score higher                    |
| `extras["source"]`    | `"graph"` (constant, source-attribution invariant)                 |
| `extras["edge_from"]` | `edge_from` (cell 0)                                               |
| `extras["edge_to"]`   | `edge_to` (cell 1)                                                 |
| `extras["edge_type"]` | `edge_type` (cell 2)                                               |
| `extras["hops"]`      | `hops` (cell 3)                                                    |
| `extras["template"]`  | the template name — useful for debug dashboards                    |

The orchestrator's `derive_graph_neighbors` reads the four `edge_*`
extras to materialise `GraphNeighbor { from, to, relation, hops }`
overlays — the contract between this lane and the overlay function
is locked to those exact keys.

Rows with empty `edge_from` or `edge_to` are dropped (defensive
projection — Nexus occasionally returns null cells for partial
matches and we don't want them surfacing as "neighbour: → ?").

### Failure handling

- Successful Cypher execution — project rows, return hits.
- Unknown template — `LaneError::Rejected("unknown graph template: ...")`.
  Cypher never sent.
- SDK error — `LaneError::Transport("execute_cypher({template}): {e}")`.
  Orchestrator's fail-open turns this into empty hits +
  `debug.errors["graph"]` populated; response stays HTTP 200.

### Acceptance for the read path

- [ ] When Nexus returns rows for a template, `results.graph_neighbors`
      populates with `{from, to, relation, hops}` matching the row cells.
- [ ] `debug.lanes.graph_ms` is `Some(_)` on every query (was missing
      because `MemoryGraphLane` returned empty).
- [ ] Unknown / arbitrary template → `LaneError::Rejected`; no Cypher
      reaches Nexus.
- [ ] Nexus down → response is HTTP 200, `debug.errors["graph"]` populated,
      `results.graph_neighbors` empty (fail-open).
- [ ] Single `Arc<NexusClient>` instance shared between `DashboardState`
      and the orchestrator's graph lane.

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

## Compat note — Nexus 1.15.0 silently drops UNWIND / `$param` writes

**Status (2026-04-27):** the live writer in `crates/cortex-graph/src/nexus_client.rs` no longer reads from `cypher/*.cypher`. Every node and edge MERGE is rendered per-row at runtime with values inline-escaped into the Cypher string (`MERGE (n:Label { id: '<escaped>' }) SET n.x = '<v>' RETURN count(*) AS written`). The writer asserts the `RETURN` produced rows so silent drops surface as real errors.

**Why.** Probing Nexus 1.15.0 directly (`POST /cypher`) showed:

| Cypher shape                                                 | Result                                |
|--------------------------------------------------------------|---------------------------------------|
| `UNWIND $rows AS row MERGE (n:L {key: row.k}) SET n += row.props` | 200 OK, **0 rows persisted, no error** |
| `MERGE (n:L {key: $k}) SET n += $props`                      | 200 OK, **0 rows persisted, no error** |
| `MERGE (n:L {key: "literal"}) SET n.x = "lit", n.y = "lit"`  | persists                               |
| `MATCH (a:L1 {key:"l1"}), (b:L2 {key:"l2"}) MERGE (a)-[r:T]->(b) RETURN r` | persists                  |
| `FOREACH (k IN [...] | MERGE …)`                             | parse error                           |
| `POST /data/nodes` typed SDK API                             | persists                               |

So Nexus 1.15 drops every write that uses `$param` substitution **inside** a write clause or that consumes an `UNWIND`-bound row. Reads with `$param` work fine. Until a Nexus version that supports the parametrised-write path ships, the writer renders Cypher with literal values escaped via `crates/cortex-graph/src/cypher.rs::cypher_str_escape`. Throughput drops (one round-trip per row vs one per batch) but a writer that lies is worse than one that's slow.

**Validated end-to-end (2026-04-27 18:22, Cortex repo bootstrap):** `MATCH (n) RETURN labels(n), count(n)` → 1833 nodes spanning Artifact / Repo / Session / Turn labels; `MATCH ()-[r]->() RETURN type(r), count(r)` → IN_REPO=2563, HAS_TURN=2. Pre-fix: 1 unlabeled node, 0 relationships.

**Re-enable the template path** once one of:

1. Nexus ships a release that supports `UNWIND $rows … MERGE …` with row-bound literals **(preferred)**.
2. We move read queries (graph traversals, `MATCH`-only) onto the registry so the file format earns its keep regardless.

See `phase1_graph_writer_nexus_compat` (archived under `.rulebook/tasks/_archive/`) for the full diagnosis and probe transcripts. The `.cypher/` files are kept as the future-state shape; the README in that directory points back here.

## Decisions

1. **MERGE-based idempotency, not lookup-then-write.** Nexus `MERGE` with unique constraints is the only thread-safe path. Never read-check-write.
2. **Natural keys where they exist; composite otherwise.** Avoid generating surrogate IDs for things that already have a natural identity (`Artifact` is the only composite case).
3. **`SIMILAR_TO` is a read-time derivation.** Storing cross-node similarities would 10× graph size and bit-rot the moment embeddings change. The query layer computes this on demand (spec 11).
4. **Cypher templates are source-controlled files, not string builds.** Security (injection) + auditability. Every template is reviewed once, used forever. **Suspended in 1.15-compat mode** — see §Compat note above.
5. **Patch coalescer lives client-side.** Nexus charges per op; coalescing before the wire saves ~40% on bootstrap.
6. **Orphans are allowed.** In distributed event streams, out-of-order arrivals are normal. Orphan `Turn{orphan:true}` nodes are a debuggable signal, not a data-loss error.
7. **Inline-literal Cypher for writes (Nexus 1.15 compat).** Render every `MERGE` per-row with values escaped into the literal via `cypher_str_escape`; refuse implicit silent successes by appending `RETURN count(*) AS written` and asserting the rows array is non-empty. Revisit once Nexus fixes parametrised writes.

## Post-write verification contract

Nexus 1.15 has two distinct silent-drop failure modes the writer **MUST** detect:

| Response shape from `RETURN count(*) AS written` | Meaning                                                | Writer action                                  |
|--------------------------------------------------|--------------------------------------------------------|------------------------------------------------|
| `rows: []` (empty)                               | UNWIND-write / `$param`-write quirk; nothing persisted | Hard error — abort the batch (`GraphClientError::Nexus`) |
| `rows: [[null]]`                                 | Successful write (Nexus suppresses real counts)        | Count as `edges_upserted` / `nodes_upserted`   |
| `rows: [[0]]` (integer zero)                     | `MATCH` found no endpoint; MERGE created nothing       | Count as `edges_dropped{edge_type=...}`, log a `WARN`, **continue the batch** |
| `rows: [[<positive integer>]]`                   | Successful write with explicit count                   | Count as upserted                              |

`assert_write_landed` in `crates/cortex-graph/src/nexus_client.rs` is the single point of enforcement. Edge silent-drops are isolated from the rest of the batch — the operator gets a structured `WARN` and the `cortex_graph_edges_dropped{edge_type}` counter, while the rest of the patch lands intact. Cross-batch endpoint races resolve themselves on the next replay because `MERGE` is idempotent.

`WriteStats.edges_upserted` is therefore a **confirmed-persisted** count, not an attempted count. Spec compliance check: in any batch report,
```
attempted_edges_for_type(t) == write_stats.edges_upserted_for_type(t)
                              + write_stats.edges_dropped.get(t).unwrap_or(0)
```
must hold. The `cortex.events.graphed` envelope inherits the confirmed count, so downstream lanes (search, dashboard) see honest numbers.

This contract was added under `phase2_graph_emit_session_and_provenance_edges` (2026-04-27) after the prior `rows.is_empty()` check let `[[0]]` responses pass through as success — 15 of 17 `HAS_TURN` edges silently lost in production with `outcome="ok"` reported. See that task's `design.md` for the full probe transcript.

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
