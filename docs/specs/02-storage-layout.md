# 02 — Storage Layout

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01
>
> Implementation: [`crates/cortex-storage/`](../../crates/cortex-storage/) — namespace constants, declarative Vectorizer/Nexus/Meili schemas, Synap stream topology, Parquet partition helpers, SQLite metadata store, SQLite-backed CAS. Runtime `ensure_*()` helpers that talk to external services live in the worker specs (04, 06, 07, 08).

## Goal

Map every Cortex artifact to a concrete physical location across the four storage backends (Vectorizer, Nexus, Synap, Meilisearch) plus the Parquet event archive, the CAS blob store, and the SQLite/Postgres metadata store. Lock the namespacing, partitioning, and retention so all downstream specs (04–08, 11) can write/read against stable addresses.

## Scope

**In:**
- Per-backend namespacing: collection names, label/edge types, index names, table names, stream names.
- Partitioning strategy (per `kind`, per `repo`, per time bucket).
- Quantization tiers and how they map to physical collections (FP32 / PQ / Binary).
- CAS layout for large blobs.
- Event archive (Parquet) layout on disk.
- Metadata store (SQLite single-node default, Postgres optional) schema overview.
- Retention sweeps and how data moves between tiers.

**Out:**
- The schema of *what gets written* to each store (→ specs 06, 07, 08).
- The classifier output schema (→ spec 05).
- Operational deployment (→ spec 03 docker-compose).

## Inputs / Outputs

### Vectorizer collections

One Vectorizer instance, multiple collections. Naming: `cortex.<kind>.<tier>`.

| Collection                       | Vectors of                  | Tier  | HNSW (M, efSearch) | Size estimate |
|----------------------------------|-----------------------------|-------|--------------------|---------------|
| `cortex.turn.fp32`               | Turn summaries (≤30 days)   | hot   | 32, 128            | ~50k vec      |
| `cortex.turn.pq`                 | Turn summaries (30–365 d)   | warm  | 16, 64             | ~500k vec     |
| `cortex.tool_call.fp32`          | Tool call summaries (hot)   | hot   | 32, 128            | ~200k vec     |
| `cortex.tool_call.pq`            | Tool call summaries (warm)  | warm  | 16, 64             | ~2M vec       |
| `cortex.code_chunk.fp32`         | Code chunks (current HEAD)  | hot   | 32, 128            | ~150k vec     |
| `cortex.code_chunk.pq`           | Code chunks (historical)    | warm  | 16, 64             | ~1M vec       |
| `cortex.doc_chunk.fp32`          | Doc chunks                  | hot   | 32, 128            | ~25k vec      |
| `cortex.decision.fp32`           | Decisions (always hot)      | hot   | 48, 256            | ~5k vec       |
| `cortex.analysis.fp32`           | Analyses (always hot)       | hot   | 48, 256            | ~1k vec       |
| `cortex.memory.fp32`             | Memory entries              | hot   | 32, 128            | ~10k vec      |
| `cortex.law.fp32`                | Law definitions             | hot   | 48, 256            | ~100 vec      |
| `cortex.cold.binary`             | Binary-quantized fallback   | cold  | 8, 32              | unbounded     |

**Embedding model:** `nomic-embed-text-v1.5` via FastEmbed (768-dim, multilingual, code-friendly). Same model across all collections so cross-collection rerank is meaningful.

**Tier transitions:** a nightly sweep (spec 06) re-encodes records whose `occurred_at` crosses a threshold: FP32→PQ at 30 days, PQ→Binary at 365 days. The CAS-stored raw payload is not touched.

### Nexus graph

One Nexus database `cortex`. Labels and relationship types frozen here; properties may evolve additively per spec 01 §"Schema evolution".

**Node labels:**

```
:Session   {session_id, tool, model, started_at, ended_at, repo, user}
:Turn      {event_id, occurred_at, content_hash, summary, tokens_in, tokens_out}
:ToolCall  {event_id, occurred_at, tool_name, outcome, duration_ms, content_hash}
:AgentCall {event_id, occurred_at, subagent_type, content_hash}
:Memory    {event_id, memory_type, name, occurred_at, content_hash}
:Decision  {event_id, title, status, occurred_at, supersedes_id, content_hash}
:Analysis  {event_id, title, status, started_at, ended_at, content_hash}
:Law       {law_id, severity, title, version, introduced_at}
:LawViolation {event_id, law_id, severity, occurred_at, evidence_ref}
:Artifact  {kind, path, content_hash, repo, language}
:Topic     {name}                    -- controlled vocab from classifier (spec 05)
:Entity    {name, kind}              -- NER (function|repo|person|package|...)
:Repo      {path, name, vcs_url}
:Model     {id}
:Tool      {id}
:User      {id}
```

**Relationship types:**

```
(Session)-[:CONTAINS]->(Turn)
(Turn)-[:INVOKED]->(ToolCall|AgentCall)
(ToolCall)-[:READ|:WROTE|:EXECUTED|:DELETED]->(Artifact)
(Turn)-[:PRODUCED]->(Memory|Decision|Analysis)
(Decision)-[:SUPERSEDES]->(Decision)
(Decision)-[:REFERENCES]->(Analysis|Memory|Artifact|Turn)
(*)-[:ABOUT {confidence}]->(Topic)
(*)-[:MENTIONS {span}]->(Entity)
(LawViolation)-[:OF]->(Law)
(LawViolation)-[:OBSERVED_IN]->(Turn|ToolCall)
(*)-[:SIMILAR_TO {score, derived_at}]->(*)   -- materialized periodically from Vectorizer KNN
(Session|Turn)-[:IN]->(Repo)
(Session|Turn)-[:USED]->(Model)
(Session)-[:VIA]->(Tool)
(Session)-[:BY]->(User)
(Artifact)-[:LIVES_IN]->(Repo)
```

**Property indexes (Cypher CREATE INDEX):**

```
:Turn(event_id), :ToolCall(event_id), :Decision(event_id)
:Artifact(content_hash), :Artifact(path)
:Topic(name), :Entity(name)
:Law(law_id)
:Session(session_id)
```

### Meilisearch indexes

One Meilisearch instance, one index per searchable kind. Per-index settings (searchable attrs, ranking rules, typo-tolerance) tuned in spec 08.

```
cortex_turns          { event_id, summary, user_message, assistant_message, topics, repo, occurred_at }
cortex_tool_calls     { event_id, tool_name, summary, command_or_input, output_excerpt, topics, repo, occurred_at }
cortex_code_chunks    { chunk_id, file_path, symbol, language, content, repo, branch, commit }
cortex_docs           { chunk_id, file_path, section_path, content, repo }
cortex_decisions      { event_id, title, body, status, topics, repo, occurred_at }
cortex_analyses       { event_id, title, body, topics, repo, occurred_at }
cortex_memories       { event_id, memory_type, name, body, occurred_at }
cortex_laws           { law_id, title, body, severity, applies_to }
```

### Synap streams and keys

```
Streams:
  cortex.events.raw          -- live ingestion bus
  cortex.events.bootstrap    -- backfill bus (lower priority, pausable)
  cortex.events.enriched     -- post-processing fan-out (dashboard, hooks, governance)
  cortex.events.invalid      -- dead-letter (failed validation)
  cortex.violations          -- governance engine output
  cortex.metrics             -- worker telemetry

Pub/Sub topics:
  cortex.live.<repo>         -- dashboard SSE per-repo
  cortex.law.<law_id>.fired  -- governance subscribers

KV namespaces (TTL'd):
  cache:query:<sha256(query+scope)>   -> serialized bundle, TTL 5 min
  cache:classify:<content_hash>       -> serialized classifier output, TTL 24 h
  cache:embed:<content_hash>          -> base64 vector, TTL 1 h (only for retry safety)
  budget:classifier:<YYYY-MM-DD>      -> running cost counter, TTL 25 h
```

### CAS (content-addressable store) for large blobs

When an envelope field exceeds 16 KB inline (spec 01 §Decisions #2), the producer writes the blob to CAS and references it by hash.

**Single-node default — SQLite blob store:**

```sql
CREATE TABLE cas_blobs (
  hash TEXT PRIMARY KEY,        -- "sha256:..."
  size INTEGER NOT NULL,
  content_type TEXT NOT NULL,   -- "text/plain", "text/x-diff", "application/json", "application/octet-stream"
  blob BLOB NOT NULL,           -- Zstd-compressed
  refcount INTEGER NOT NULL DEFAULT 0,
  first_seen TEXT NOT NULL,
  last_referenced TEXT NOT NULL
);
```

Workers increment/decrement `refcount` as they (un)reference blobs. A weekly `vacuum` job deletes blobs with `refcount=0` and `last_referenced > 30 days ago`.

**Distributed deployment — MinIO:** identical contract, blobs go to a single bucket `cortex-cas` keyed by hash; `cas_blobs` table keeps the metadata only.

### Event archive (Parquet)

Every event published to any `cortex.events.*` stream is durably written to Parquet for replay, audit, and bootstrap re-runs.

**Layout:**

```
cortex-data/events/
  ├─ year=2026/month=04/day=17/hour=12/
  │   ├─ raw-00000.parquet
  │   ├─ raw-00001.parquet
  │   └─ bootstrap-00000.parquet
  └─ ...
```

Hourly rotation; Zstd compression (level 6); columnar layout with `event_id`, `kind`, `tool`, `repo`, `occurred_at` as native columns and `payload` / `context` as JSON strings (pruned via DuckDB / Polars when needed).

**Rollups:**
- After 90 days: hourly files merged into one **daily** file.
- After 365 days: daily files merged into one **monthly** file.
- After 3 years: monthly files dropped unless `pii_risk = "low"` or `kind` ∈ {`decision`, `analysis`, `law_violation`}.

### Metadata store (SQLite default, Postgres optional)

Operational data that doesn't fit a graph or a vector store:

```sql
-- session lifecycle
CREATE TABLE sessions (session_id TEXT PRIMARY KEY, tool, model, repo, user, started_at, ended_at, event_count, ...);

-- repo registry
CREATE TABLE repos (path TEXT PRIMARY KEY, name, vcs_url, last_bootstrapped_at, last_synced_at, config_json);

-- bootstrap progress
CREATE TABLE bootstrap_jobs (job_id TEXT PRIMARY KEY, repo_path, started_at, finished_at, files_processed, chunks_emitted, status);

-- classifier budget tracking
CREATE TABLE classifier_spend (day TEXT PRIMARY KEY, calls, tokens_in, tokens_out, est_usd_cents);

-- law registry (mirrors disk for fast queries)
CREATE TABLE laws (law_id TEXT PRIMARY KEY, version, severity, title, body, detector_path, introduced_at, retired_at);

-- trust scores
CREATE TABLE trust_scores (model TEXT, repo TEXT, score REAL, computed_at, PRIMARY KEY(model, repo));

-- retention bookkeeping
CREATE TABLE retention_sweeps (sweep_id TEXT PRIMARY KEY, started_at, finished_at, records_demoted, records_dropped, tier_transitions_json);

-- API keys / users (basic; defer richer RBAC to Vectorizer/Nexus native)
CREATE TABLE api_keys (key_hash TEXT PRIMARY KEY, name, scopes, created_at, last_used_at);
```

## Design

### Where each kind lands (cross-store summary)

| Event kind        | Vectorizer collection         | Nexus label   | Meilisearch index    | CAS? | Archive? |
|-------------------|-------------------------------|---------------|----------------------|:----:|:--------:|
| `turn`            | `cortex.turn.{fp32\|pq}`      | `:Turn`       | `cortex_turns`       | rare |    ✓     |
| `tool_call`       | `cortex.tool_call.{fp32\|pq}` | `:ToolCall`   | `cortex_tool_calls`  | freq |    ✓     |
| `agent_call`      | `cortex.tool_call.fp32`*      | `:AgentCall`  | `cortex_tool_calls`* | freq |    ✓     |
| `memory`          | `cortex.memory.fp32`          | `:Memory`     | `cortex_memories`    | rare |    ✓     |
| `decision`        | `cortex.decision.fp32`        | `:Decision`   | `cortex_decisions`   | some |    ✓     |
| `analysis`        | `cortex.analysis.fp32`        | `:Analysis`   | `cortex_analyses`    | some |    ✓     |
| `law_violation`   | `cortex.tool_call.fp32`*      | `:LawViolation`| (n/a, link via Nexus)| no  |    ✓     |
| `artifact:code`   | `cortex.code_chunk.{fp32\|pq}`| `:Artifact`   | `cortex_code_chunks` | rare |    ✓     |
| `artifact:doc`    | `cortex.doc_chunk.fp32`       | `:Artifact`   | `cortex_docs`        | rare |    ✓     |

\* shares a collection/index for retrieval purposes; `kind` distinguishes via filter.

### Quantization & tier sweep

Daily job (`cortex-retention sweep`) runs against Vectorizer collections:

```
for each collection in {turn, tool_call, code_chunk}:
    SELECT vectors WHERE occurred_at < now - 30d AND tier = 'fp32'
    -> re-encode with PQ
    -> insert into cortex.<kind>.pq
    -> delete from cortex.<kind>.fp32

    SELECT vectors WHERE occurred_at < now - 365d AND tier = 'pq'
    -> re-encode with binary quantization
    -> insert into cortex.cold.binary (with kind tag)
    -> delete from cortex.<kind>.pq
```

Same sweep also applies retention rules from spec 01 (PII tiers): `pii_risk=high` raw payloads are dropped at 30d (CAS blobs deleted, Parquet event row blanked-but-kept-for-audit), `medium` at 90d (re-summarized), `low` indefinite.

### Idempotency contracts

- **Vectorizer:** upsert by `event_id` (primary key in collection metadata); re-running embed for the same event is a no-op.
- **Nexus:** `MERGE` on `(:Label {event_id})`; relationship creation uses `MERGE` with full edge signature.
- **Meilisearch:** primary key = `event_id` (or `chunk_id` for code/docs).
- **CAS:** insert-or-ignore on `hash`; refcount only incremented if reference is new.
- **Parquet:** files are hourly-immutable; replay produces identical bytes given same input order.

### Backup & restore

- Vectorizer: native snapshots API (~25 § features), write to `cortex-data/snapshots/vectorizer/`.
- Nexus: built-in WAL + snapshot.
- SQLite metadata: simple file copy with `VACUUM INTO`.
- Synap: `BGSAVE`-equivalent, optional given streams are not authoritative.
- CAS: file/bucket-level mirror.
- Parquet archive: source of truth — losing it loses replay capability but not query capability.

A `cortex backup` CLI bundles all five into one timestamped directory.

## Acceptance criteria

- [ ] All Vectorizer collections listed above can be created via Vectorizer's REST/MCP API by a setup script.
- [ ] All Nexus labels and relationship types are creatable, and the property indexes exist.
- [ ] All Meilisearch indexes are creatable with their primary keys.
- [ ] All Synap streams/topics/KV namespaces are addressable and a smoke test publishes/reads from each.
- [ ] CAS roundtrip: write a 5 MB blob, read by hash, refcount increments correctly, vacuum drops it after refs hit zero.
- [ ] Parquet write + read with DuckDB returns the same row count as ingested.
- [ ] SQLite metadata schema migrates cleanly from empty (using a single `schema.sql` file).
- [ ] Daily retention sweep moves a sample event FP32 → PQ at the 30-day boundary in test mode (--time-travel flag).
- [ ] `cortex backup && cortex restore` on a fresh node reproduces query results bit-for-bit.

## Decisions

1. **Embedding model:** `nomic-embed-text-v1.5` (768-dim) across all collections — multilingual, code-aware, FastEmbed-supported.
2. **Single Nexus database, not per-repo.** Cross-repo queries are critical (Topic/Entity/Decision span repos). `:Repo` label + `[:IN]` edge handles per-repo scoping.
3. **CAS default is SQLite blob store**, not separate object store. MinIO only when sharding/multi-node is required.
4. **Parquet is the durable archive, not Synap.** Synap streams are best-effort; Parquet is the recovery source. Consequence: Parquet-write must succeed before ingestion ack — synchronous-but-batched (250 ms windows).
5. **Quantization tiers are physical separate collections,** not in-place compression. Lets us tune HNSW per tier and drop cold tiers wholesale if needed.

## Open questions

*(none — defaults locked; reopen by superseding spec)*

## References

- Architecture §3 (ecosystem), §6 (bootstrap), §7 (component boundaries), §11 (resource budget).
- Spec 01 — Event schema (`event_id`, `content_hash`, `redactions`, inline-CAS rule).
- Spec 03 — Local stack (provisions these stores).
- Specs 06, 07, 08 — concrete writers for each backend.
- Vectorizer docs: collection management, PQ, MMap, snapshots.
- Nexus docs: openCypher subset, indexes, KNN.
- Meilisearch docs: index settings, primary key, ranking rules.
