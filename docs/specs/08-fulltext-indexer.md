# 08 — Full-text indexer (Meilisearch client)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 02

## Goal

Index every enriched event's searchable text into **Meilisearch** with typo tolerance, faceted filters, and snippet highlighting — so the query API (spec 11) has a fast keyword lane alongside the vector and graph lanes. This spec is the client: index configuration, document mapping, batched writes, idempotency. Meilisearch operations (install, backup) are out of scope.

## Scope

**In:**
- Worker consuming `cortex.events.enriched` + `cortex.events.embedded`.
- Meilisearch index layout (one index per collection family; mirrors embedder's collection boundaries).
- Document schema: shared core + per-kind extensions.
- Searchable/filterable/sortable attribute configuration.
- Client: HTTP batched upsert, retry, backoff, idempotent upsert.
- Dedup by `document_id` (same content → no-op write).
- Synonyms, stop-words, and ranking-rules configuration (versioned).
- Telemetry.

**Out:**
- Query surface (spec 11 — read path is the orchestrator's job).
- Index analytics / query-log mining — future.
- Meilisearch deploy/ops (docker-compose in spec 03).
- Eventual migration to Lexum — tracked as an open question in architecture §12.

## Inputs / Outputs

### Trait

```rust
#[async_trait]
pub trait FulltextIndexer: Send + Sync {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport>;
}

pub struct IndexReport {
    pub documents_upserted: u32,
    pub documents_deduped: u32,                  // same doc_id + same content_hash already present
    pub by_index: BTreeMap<String, u32>,
    pub latency_ms: u32,
}
```

### Document schema

Shared core (present on every document):

```jsonc
{
  "id": "<doc_id>",                              // see identity section
  "event_id": "01HXYZ...",
  "kind": "tool_call.edit",
  "content_hash": "sha256:...",
  "ts": 1713369600000,                            // ms since epoch (filterable & sortable)
  "repo": "Vectorizer",
  "path": "src/index/hnsw/mod.rs",
  "topics": ["code", "refactor"],
  "severity": "notable",
  "pii_risk": "low",
  "summary": "Refactored HNSW ef_search...",      // classifier summary if present
  "title": "hnsw_ef_search",                      // symbol for code, H1 for docs, first 80 chars otherwise
  "body": "pub fn hnsw_ef_search(k: usize) -> ... ",  // the primary searchable text
  "language": "rust"
}
```

Per-kind extensions live under `ext.<kind>`:

```jsonc
"ext": {
  "tool_call": { "tool_name": "Edit", "status": "ok" },
  "decision": { "status": "accepted", "supersedes": "DEC-0042" },
  "law_violation": { "law_id": "LAW-007", "tier": 3 }
}
```

Missing extensions are absent — no null-padding.

### Indexes (per-kind family, per-repo, mirrors spec 06)

| Family suffix        | Holds                          | Typical size (live, post-bootstrap, per repo) |
|----------------------|--------------------------------|-----------------------------------------------|
| `code`               | `tool_call.*`, `artifact.code` | ~30 MB                                         |
| `docs`               | `artifact.doc`, docs chunks    | ~25 MB                                         |
| `decisions`          | `decision.*`                   | ~1 MB                                          |
| `turns`              | `turn.*`                       | ~50 MB                                         |
| `governance`         | `law`, `law_violation`         | ~1 MB                                          |
| `misc`               | everything else                | varies                                         |

The full index uid is `{prefix}-{repo_slug}-{family}` (default prefix `cortex-`). Per-project isolation is mandatory: a Cortex repo populates `cortex-cortex-docs` / `cortex-cortex-code`, while a Tml repo populates `cortex-tml-docs` / `cortex-tml-code`. Events with no `context.repo` route to the `unknown` slug. The slug is canonicalised through `cortex_storage::names::slug_for_repo` (lowercase ASCII, non-`[a-z0-9-]` collapsed to `-`, no leading/trailing dashes).

### Routing matrix

The family suffix is picked deterministically from `(kind, classifier.topics, context.path)` by `cortex_fulltext::routing::family_for_event`. Order is significant — the first rule that matches wins.

| Predicate                                                          | Family         |
|--------------------------------------------------------------------|----------------|
| `kind == decision`                                                 | `decisions`    |
| `kind == law_violation` (or, when introduced, `kind == law`)       | `governance`   |
| `kind == turn` ∨ `kind == agent_call`                              | `turns`        |
| `kind == tool_call`                                                | `code`         |
| `kind == artifact` ∧ `path` ends with `CODE_EXTENSIONS`            | `code`         |
| `kind == artifact` ∧ `path` ends with `DOC_EXTENSIONS`             | `docs`         |
| `kind == artifact` ∧ `topics ⊇ {code}`                             | `code`         |
| `kind == artifact` ∧ `topics ⊇ {doc, documentation}`               | `docs`         |
| anything else (including `memory`, `analysis`, signal-less artifacts) | `misc`      |

`CODE_EXTENSIONS` is the curated allowlist `rs, ts, tsx, js, jsx, mjs, cjs, vue, py, go, rb, java, kt, scala, c, cc, cpp, h, hpp, cs, swift, php, lua, sh, bash, zsh, ps1, fish, sql, proto` (`.vue` added 2026-05-01 per issue #3 — Vue SFCs were previously routed to `misc`). `DOC_EXTENSIONS` is `md, mdx, markdown, rst, adoc, asciidoc, txt, rtf, tex, org`. Files with neither extension and no topic signal land in `misc` rather than silently piling into `docs` (the 2026-04-27 audit failure mode).

**Tie-break for artifacts with mixed `topics: [code, doc]`:** the path extension always wins because the file itself is the most reliable signal. Only when the extension is unknown does the classifier's topic list arbitrate.

**Observability:** every routed envelope increments `cortex_fulltext_routed_total{index="<full-uid>"}` (see §Observability). The post-bootstrap operator check is "every of the six families is non-zero, and `misc` stays in the low single-digit percent of total throughput."

### Identity

```
doc_id = event_id                              // live events
doc_id = "bootstrap:" + repo + ":" + path + ":" + content_hash   // bootstrap artifacts
```

- `doc_id` is stable across re-runs.
- Meilisearch upsert is `doc_id`-keyed, so retries and bootstrap replays are no-ops.

## Design

### Index configuration (per index, set at `ensure_index`)

```jsonc
{
  "searchableAttributes": ["title", "body", "summary", "path", "topics", "ext.tool_call.tool_name"],
  "filterableAttributes": ["kind", "repo", "path", "topics", "severity", "pii_risk", "ts", "language",
                           "ext.decision.status", "ext.law_violation.law_id"],
  "sortableAttributes":   ["ts"],
  "displayedAttributes":  ["*"],
  "rankingRules": [
    "words",
    "typo",
    "proximity",
    "attribute",
    "sort",
    "exactness",
    "ts:desc"                                    // newer > older, all else equal
  ],
  "stopWords": ["a","an","the","of","in","to","and","or"],
  "synonyms": {
    "refactor": ["rewrite", "refactoring"],
    "bug":      ["issue", "defect"],
    "law":      ["policy", "rule"]
  },
  "typoTolerance": {
    "enabled": true,
    "minWordSizeForTypos": { "oneTypo": 4, "twoTypos": 8 }
  }
}
```

Synonyms and stop-words live in `cortex-workers/meilisearch/settings.v<N>.json`, versioned. Changing them triggers a **settings bump** but not a reindex — Meilisearch re-ranks on the fly.

### Meilisearch client

```rust
pub struct MeiliClient {
    endpoint: Url,                           // http://localhost:7700
    api_key: String,
    http: reqwest::Client,
    retry: RetryPolicy,                      // 3 attempts, exp backoff 100/400/1600 ms
}

impl MeiliClient {
    async fn ensure_index(&self, name: &str, settings: &IndexSettings) -> Result<()>;
    async fn upsert_documents(&self, index: &str, docs: &[Document]) -> Result<TaskUid>;
    async fn wait_task(&self, task: TaskUid, timeout: Duration) -> Result<TaskStatus>;
}
```

- **Async task model:** Meilisearch upserts are async (returns a `taskUid`). For low-volume live traffic the worker **fire-and-forgets**; for bootstrap it **waits** to detect failures early (flag `CORTEX_FULLTEXT_AWAIT_TASK=1`).
- **Batch size:** 1 000 documents per upsert call. Meilisearch is throughput-friendly; the bottleneck is the HTTP round-trip, not document parsing.
- **Idempotency:** `doc_id`-keyed; dupes are overwrites (same bytes → same result).

### Per-kind document builder

`cortex-workers/src/fulltext/doc_builder.rs` has one function per event family:

```rust
fn build_tool_call_doc(e: &EnrichedEvent) -> Document { ... }
fn build_decision_doc(e: &EnrichedEvent) -> Document { ... }
fn build_turn_doc(e: &EnrichedEvent) -> Document { ... }
// ...
```

Each builder is pure (no I/O) and trivially unit-testable. The dispatcher is an exhaustive match on `kind` — adding a new kind is a compile error until a builder exists.

### Body selection (what goes into `body`)

The `body` field is the primary searchable text. Selection priority:

1. If the event has a `classifier.summary` **and** raw payload > 4 KB → use the summary (same rule as spec 06 — keeps cost bounded).
2. Otherwise, use the **redacted raw payload text** (for tool_call: `input.command` or `input.text`; for turn: the user prompt; for artifact: the chunk text from spec 06).
3. If neither is present, skip the event with `counter: fulltext.skipped_empty`.

The raw payload is **never** indexed without redaction.

### Read-side projection (phase6g)

When [`cortex_api::meili_lane`](../../crates/cortex-api/src/meili_lane.rs)
projects a Meili hit back into a `LaneHit` for the orchestrator,
the field that becomes `LaneHit.text` is **kind-aware**. The
pre-phase6g chain (`summary > title > body`) was wrong for
`kind=artifact`: code/doc files have `summary = ""`,
`title = path`, `body = real content`, so the chain stopped at
`title` and every artifact hit landed with `text = "<path>"`,
masking the actual file content. Closes
[F-009](../analysis/relevance/01-findings.md#f-009--meili-artifact-projection-prefers-path-over-body).

| `doc.kind`                           | Precedence for `LaneHit.text`     |
| ------------------------------------ | --------------------------------- |
| `artifact`, `law_violation`          | `body > summary > title`          |
| `decision`, `analysis`, `memory`     | `summary > title > body`          |
| `turn`, `tool_call`, `agent_call`    | `summary > body > title`          |
| _anything else / `None`_             | `summary > title > body`          |

`LaneHit.path` is populated separately, so demoting `path` from
`text` does not lose information — it just stops it from masking
the body. Bodies above `4 KiB` emit a `tracing::debug!` so
operators can flag oversized chunks; the orchestrator's existing
trim ladder enforces the per-snippet byte cap.

The fulltext-worker write path also gained an additive guard
(`crates/cortex-workers/src/fulltext/builders.rs`): when
`select_body` produces text but `body`, `summary`, AND `title` all
end up empty after derivation, `tracing::warn!` records the
`event_id` + `content_hash` so the upstream emitter can be traced.
The doc is still written — the warn is informational, not a write
gate.

### Cross-backend consistency doctor (phase4d)

`cortex-ops doctor-consistency` runs out-of-band (operator command, not part of the worker boot path) and reports per-`(repo, family)` coverage across the event archive, Meilisearch, Vectorizer, and Nexus. The query-overlap probe mode is carved out into `phase4i_doctor_query_overlap_mode`.

Probes:

- **Archive** — walks `$CORTEX_ARCHIVE_ROOT/events/**/raw-*.parquet` (zstd-NDJSON), routes every envelope through `cortex_fulltext::routing::family_for_event` so the partition view exactly matches what the live indexer produces, and aggregates `(repo_slug, family) → envelope_count`. Envelopes without a `kind` or `context.repo` are dropped silently.
- **Meili** — wraps `cortex_fulltext::MeiliClient::list_indexes` and lifts every canonical `cortex-{repo_slug}-{family}` name into the same partition map. Names that fail `is_canonical_index_name` land in a separate sweep-candidate list (informational; phase4a's boot-time sweep is the actor that drops them).
- **Vectorizer** (phase4h) — authenticates via `POST /auth/login` (`CORTEX_EMBEDDER_VECTORIZER_USER` / `_PASSWORD`), calls `list_collections()`, parses `cortex-{repo_slug}-{family}` names, and aggregates `(repo_slug, family) → vector_count`. Non-canonical collection names land in a sibling sweep-candidate list. The probe runs only when both URL and credentials are set.
- **Nexus** (phase4h) — wraps the same `LiveNexusClient` the writer uses and runs `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN r.name AS repo, count(a) AS artifacts`. The graph is repo-grain only, so the row's `nexus_artifacts` value repeats for every `(repo, *)` partition that shares a `repo`. Probe runs only when `--nexus` (or `$CORTEX_NEXUS_URL`) is set.

Coverage policy:

- A row is marked **inconsistent** when `archive_events > 0` AND Meili lacks the matching index OR has zero docs in it.
- A row is marked **suspicious** (but not `inconsistent`) when both `meili_docs > 0` and `vec_vectors > 0` and `vec_vectors / meili_docs > vec_to_meili_ratio_max` (default `50`). Chunking can legitimately multiply but a 50× expansion still warrants a manual look. Suspicious rows do **not** flip `failed`.
- Meili-only rows (archive empty, Meili populated) are **informational** — they surface in the table but don't fail the run; they're the expected shape after archive rotation or bootstrap-only ingestion.
- The CLI exits non-zero only when any row is `inconsistent`. JSON output (`--json`) emits the full `DoctorReport` shape with the new `vec_vectors`, `nexus_artifacts`, `suspicious`, and `non_canonical_vectorizer_collections` fields.

Re-running the doctor after a fix proves the fix at the data level, not just the unit-test level. Live runs against the dev cluster have already caught real drift (`gui/code`, `gui/turns`, `rust/code` partitions present in the archive but missing from Meili after the phase4a sweep).

#### Probe mode (phase4i)

`cortex-ops doctor-consistency --query <q>` (repeatable) extends the
coverage report with **per-query overlap** between the three lanes.
For each query the doctor:

1. Runs the same text against Meili (`POST /indexes/{uid}/search`,
   fanning out across every canonical index produced by the coverage
   probe), Vectorizer (`search_vectors` over every canonical
   collection), and Nexus (Cypher `CONTAINS` substring on
   `Artifact.body`).
2. Dedupes each lane's hits into a top-K result-path set (`path` is
   read from the lane's hit metadata; falls back to the document id
   when the lane omits an explicit `path`).
3. Computes pairwise Jaccards `|A ∩ B| / |A ∪ B|` for the three
   lane pairs plus the cardinality of the three-way intersection.

Threshold policy: `--min-overlap-jaccard` (default `0.2`) flips the
report's `failed` flag when **any** pair's Jaccard falls below the
bound AND both involved lanes returned at least one path. (One-empty
pairs are silenced — the partition-coverage doctor already owns the
"this lane is empty" signal.) The CLI exits non-zero on `failed`.

Rendered output for each query is a Markdown block with the per-pair
table:

```
### Query: <q>

k=<k>, triple_intersection=<n>

| pair | jaccard | a_size | b_size | intersection |
|------|--------:|-------:|-------:|-------------:|
| vec/meili | 0.823 | 10 | 10 | 7 |
| vec/nexus | 0.118 | 10 | 8 | 1 |     <- below 0.2 threshold
| meili/nexus | 0.157 | 10 | 8 | 1 |   <- below 0.2 threshold

**FLAG:** pair(s) below threshold 0.2: vec/nexus=0.12, meili/nexus=0.16
```

Implementation lives in
[`cortex_ops::probe`](../../crates/cortex-ops/src/probe.rs) (trait +
Jaccard math + memory probe) and the live HTTP/SDK fan-out lives in
the binary's `LiveMeiliQueryProbe` / `LiveVectorizerQueryProbe` /
`LiveNexusQueryProbe`.

#### CI gate (phase4j)

The doctor runs in CI on every push / PR via
[`.github/workflows/doctor.yml`](../../.github/workflows/doctor.yml).
The workflow brings up the same `docker-compose` stack the local dev
loop uses (Vectorizer + Nexus + Synap + Meilisearch), seeds three
synthetic temp repos (`alpha` / `beta` / `gamma`) through
`cortex-bootstrap --workspace`, then runs
`cargo run -p cortex-ops -- doctor-consistency --json` and uploads
the JSON report as the `doctor-consistency-report` workflow
artifact. The workflow fails when the doctor exits non-zero — i.e.,
when at least one row is `inconsistent` or any `--query` overlap
falls below the configured threshold.

The local equivalent is `make doctor-consistency`. The Makefile
target's env-var contract is documented inline (see the comment block
above the `doctor-consistency:` rule). Operators run it after a
schema migration or a backend swap to confirm nothing drifted under
their feet.

### Boot-time stale-index sweep (phase4a)

Before the worker pool starts pulling from `cortex.events.enriched`,
`main.rs` runs `sweep_stale_indexes` once against the configured
Meili cluster. The sweep walks every existing index and applies
this policy:

- **Canonical name** — `cortex-{repo_slug}-{family}` where
  `family ∈ {code, docs, decisions, turns, governance, misc, analyses}`
  and `repo_slug` matches `[a-z0-9_]([a-z0-9_-]*[a-z0-9_])?` —
  preserved unconditionally.
- **Non-canonical & empty** — dropped via `delete_index`. Logged at
  `info` with `reason="stale-naming"`. The 2026-04-27 audit's seven
  offenders (`cortex-code`, `cortex-decisions`, `cortex-docs`,
  `cortex-governance`, `cortex-misc`, `cortex-turns`, plus the
  post-audit `cortex-analyses`) all fell into this bucket.
- **Non-canonical & non-empty** — preserved. The worker emits
  exactly one `warn` line naming the index and its document count
  so the operator can manually triage; no automatic deletion ever
  drops state.

The sweep is fully idempotent — re-running after a successful
sweep examines fewer indexes (the deleted ones are gone) and
produces zero deletions on subsequent runs.

The matcher lives in [`cortex_fulltext::routing::is_canonical_index_name`](../../crates/cortex-fulltext/src/routing.rs) and is exercised by `index_name`'s `debug_assert!`, so writer drift is caught in dev / CI before reaching production.

The legacy `for family in FAMILIES { ensure_index(format!("cortex-{family}"), ...) }` boot loop that produced the stale names was removed in phase4a — per-project uids materialise lazily on first upsert via `MeiliFulltextIndexer::ensure_settings`, so eager bootstrap of the un-slugged family set was always orphaning state into Meili.

### Boot-time replay-missing partitions (phase4f)

After the stale-sweep but before the worker pool starts pulling from
`cortex.events.enriched`, `main.rs` may run `replay_missing_partitions`
once against the configured Meili cluster. The routine closes the
recovery hole left after phase4a: even with sweep + lazy `ensure_index`
in place, the worker still depends on the Synap stream catching every
event in real time. If the worker crashes before catching up to a
bootstrap, the Synap stream rotates past the gap, or the stack starts
cold against an archive-only deployment, the corresponding
`cortex-{repo_slug}-{family}` partitions never materialise and the
keyword lane silently degrades to "missing repo".

The routine:

1. Walks Meili via `list_indexes` and parses every canonical-shaped
   uid back into its `(repo_slug, family)` pair.
2. Scans `$CORTEX_ARCHIVE_ROOT` (`raw-*.parquet` zstd NDJSON, the
   same path `cortex-graph-backfill` uses) and computes the union
   of `(repo_slug, family)` pairs every envelope would route to.
3. Replays only the envelopes whose pair is in the set difference
   (archive minus Meili) through the production
   `MeiliFulltextIndexer::index_batch` upsert path.

Idempotent because Meili keys on the document id derived from
`content_hash` — re-running on a populated cluster is a no-op.

**Off by default** — gated on `CORTEX_FULLTEXT_REPLAY_MISSING=1` so a
hot-path restart never triggers a multi-minute archive scan. When
enabled, the routine reads the archive root from `CORTEX_ARCHIVE_ROOT`
(or the `~/.cortex/archive` fallback) and the index prefix from the
existing `FulltextConfig`. Errors are logged at `warn` and the boot
proceeds without replay; failure to replay never blocks live traffic.

Per-partition observability lands in
`cortex_fulltext_replay_events_total{repo, family}`. The boot path
emits exactly one `info` summary at the end of the phase:

```
fulltext replay-missing complete
  examined_archives=<N>
  missing_partitions=<M>
  replayed_events=<K>
  latency_ms=<L>
```

Implementation in [`cortex_fulltext::boot_replay`](../../crates/cortex-fulltext/src/boot_replay.rs).

### Worker concurrency

```
Synap consumer ──▶ doc builder pool ──▶ Meili upsert pool ──▶ publisher (cortex.events.fulltext_indexed)
```

**Knobs (env):**
- `CORTEX_FULLTEXT_WORKERS=4`
- `CORTEX_FULLTEXT_BATCH=1000`
- `CORTEX_FULLTEXT_FLUSH_MS=1000`
- `CORTEX_FULLTEXT_AWAIT_TASK=0` (1 during bootstrap)

Backpressure: Meilisearch returns 503 when overloaded; the worker backs off and pauses the consumer after sustained 503s for >30 s.

### Failure modes

| Failure                           | Handling                                                                 |
|-----------------------------------|--------------------------------------------------------------------------|
| Meili 503 / rate limit            | Retry with backoff; eventually pause consumer                            |
| Meili 400 (schema error, bad id)  | Fail event → `cortex.events.invalid` with cause                          |
| Task failed (`wait_task` == failed) | Record `fulltext.task_failures`; dead-letter the batch                  |
| Settings incompatible (new rankingRules conflict) | Fail fast at startup with clear message                  |
| Empty body + no summary           | Skip; counter `fulltext.skipped_empty`                                   |
| Oversize single document (>10 MB) | Truncate `body` to 10 MB, set `truncated=true`; counter increments        |

### Observability

```
cortex.fulltext.documents.total       counter, labels: index
cortex.fulltext.batch.size            histogram
cortex.fulltext.upsert.latency_ms     histogram, labels: index
cortex.fulltext.dedup.hits            counter
cortex.fulltext.task_failures         counter, labels: reason
cortex.fulltext.errors                counter, labels: status
cortex.fulltext.skipped_empty         counter
cortex.fulltext.truncated             counter
cortex.fulltext.backpressure.active   gauge
cortex.fulltext.replay_events_total   counter, labels: repo, family
```

## Read path

The write path above is consumed at query time by `cortex-api`'s
keyword lane (spec 11 §Lane traits). The lane translates the
orchestrator's `KeywordRequest { index, query, limit, scope }` into
a Meilisearch `POST /indexes/{uid}/search` body and projects each
returned hit into the `LaneHit` shape the fusion stage expects.

### Live lane: `MeiliKeywordLane`

Lives at [crates/cortex-api/src/meili_lane.rs](../../crates/cortex-api/src/meili_lane.rs).
Selected at daemon startup when `CORTEX_FULLTEXT_MEILI_URL` is set
**and** `/health` answers a 2xx within the probe timeout. On any
failure (env unset, server unreachable, probe timeout, build error)
the daemon falls back to `MemoryKeywordLane` so cold-stack dev keeps
working — the failure is logged at WARN with the URL + reason.

**Search request shape** (per [Meili search API](https://www.meilisearch.com/docs/reference/api/search)):

```json
POST /indexes/{uid}/search
{
  "q": "<request.query verbatim>",
  "limit": <request.limit>,
  "showRankingScore": true
}
```

`showRankingScore=true` surfaces the per-hit `_rankingScore` (a
normalised `[0, 1]` float) on every hit, which the lane projects
into `LaneHit.score`. Without this the lane would have to fall back
to the positional `1/(60+rank)` artefact the test double produces
— RRF fusion would still work but the score column on the snippet
becomes meaningless to downstream callers.

**HTTP status handling:**

- `2xx` — parse `hits[]` and project each into a `LaneHit` (see below).
- `404` — return `Vec::new()`. Per-project indexes (`cortex-{slug}-{family}`)
  are materialised lazily by the worker on first upsert; a 404 is the
  legitimate empty-index case, not an error.
- `4xx` (other) / `5xx` — return `LaneError::Rejected(detail)`. The
  orchestrator's fail-open policy turns this into an empty hit set
  plus a `debug.errors["keyword"]` entry; the response stays HTTP 200.
- Transport / decode failure — `LaneError::Transport`. Same fail-open
  semantics as above.

### Hit projection (Meili → `LaneHit`)

| LaneHit field   | Meili source                                                 |
|-----------------|--------------------------------------------------------------|
| `doc_id`        | `meili|{index}|{event_id or id or "unknown"}`               |
| `text`          | `summary` → `title` → `body` (first non-empty wins)          |
| `repo`          | `repo`                                                       |
| `path`          | `path`                                                       |
| `symbol`        | `kind` (e.g. `"turn"`, `"decision"`, `"law_violation"`)      |
| `content_hash`  | `content_hash`                                               |
| `score`         | `_rankingScore` (defaults to `0.0` when absent)              |
| `ts`            | `ts` (defaults to `0` when absent)                           |
| `severity`      | `severity`                                                   |
| `extras["source"]` | `"keyword"` (constant — the source-attribution invariant) |

The `summary → title → body` fallback chain matches the deterministic
body-selection rule in §Document body. The lane never blends the
three; it picks the first non-empty projection. Empty `text` only
surfaces when the document carries none of the three.

### Source-attribution invariant

Every hit produced by the keyword lane MUST carry
`extras["source"] = "keyword"`. The orchestrator's `lane_label()`
([crates/cortex-api/src/orchestrator.rs](../../crates/cortex-api/src/orchestrator.rs))
reads this field; when it's missing, the label falls back to
`"vector"` — the 2026-04-27 audit caught this regression in the
`MemoryKeywordLane` test double (every keyword hit surfaced as
`source: "vector"` on the snippet column).

A `debug_assert!` in `Orchestrator::run` walks the keyword lane's
result and panics in debug builds when any hit lacks the marker.
Both the live `MeiliKeywordLane` and the `MemoryKeywordLane`
seeders (`archive_loader`, `meili_loader`) stamp the field.

### Configuration

| Env var                          | Default              | Purpose                                                      |
|----------------------------------|----------------------|--------------------------------------------------------------|
| `CORTEX_FULLTEXT_MEILI_URL`      | (unset)              | Live lane base URL. Unset → MemoryKeywordLane fallback.      |
| `CORTEX_FULLTEXT_MEILI_API_KEY`  | (unset)              | Bearer token. Sent on `/health` probe + every search call.   |

### Acceptance for the read path

- [ ] Two `/v1/query` calls with distinct `query` strings against the
      same index return distinct `results.snippets` sets (the
      `MemoryKeywordLane` regression returned the same 5 envelopes
      regardless of query).
- [ ] A nonsense query (`"asdfqwerty12345"`) returns either an empty
      `results.snippets` or only fuzzy-but-relevant matches per
      `typoTolerance` settings.
- [ ] Meili down → response is HTTP 200, `debug.errors["keyword"]`
      populated, `results` may be empty (fail-open).
- [ ] Snippet `source` column is `"keyword"` for every hit produced
      by the keyword lane (no `"vector"` mislabelling).

## Acceptance criteria

- [ ] Startup against an empty Meilisearch creates all indexes with the declared settings; idempotent on re-run.
- [ ] 10 000-event synthetic stream lands in the correct indexes (per-kind split); counts match by kind.
- [ ] Idempotency: replaying the same 10 000 events results in zero new documents (dedupe hits == documents).
- [ ] Bootstrap identity: a doc_id like `bootstrap:Vectorizer:src/lib.rs:<hash>` remains stable across two bootstrap runs.
- [ ] Body fallback: a 20 KB payload with a classifier summary uses the summary as `body`; `truncated` is false.
- [ ] Body fallback: a 20 KB payload **without** a summary truncates at 10 MB-boundary isn't reached but the raw path is still stored (`summary` null, `body` contains full redacted text).
- [ ] Typo tolerance: query "refator" returns documents containing "refactor" within the first 10 hits on the `cortex-code` index with a 1 000-doc sample.
- [ ] Synonyms: query "bug" returns docs mentioning "defect" in ranked results.
- [ ] Filterable: `filter: 'repo = Vectorizer AND severity = critical'` returns only matching docs.
- [ ] Sortable: `sort: 'ts:desc'` orders results newest-first on identical-score ties.
- [ ] Settings bump (v1 → v2) updates ranking rules without reindexing; counter `fulltext.settings_bump` increments.
- [ ] Meilisearch 503 soak: 1-minute rate-limit, zero event loss, drains after the storm.
- [ ] Schema drift: pre-existing incompatible settings fail at startup with a clear message.
- [ ] Telemetry non-zero after soak; P95 upsert latency < 300 ms on dev stack with local Meilisearch.

## Decisions

1. **Per-kind indexes, not one mega-index.** Matches embedder collections (spec 06) and lets us tune ranking rules per family (code ranks differ from decisions).
2. **Meilisearch, not Lexum, in v1.** Decision ratified in architecture §12; Lexum is not production-ready. Swap is a client-side migration (write a `LexumClient` implementing the same trait) — no schema changes upstream.
3. **Fire-and-forget in live mode, await in bootstrap.** Live traffic can tolerate async task processing; bootstrap needs fail-fast to avoid silent corpus corruption.
4. **Synonyms + stop-words versioned.** They're part of the retrieval contract; changing them must be auditable.
5. **Body selection is deterministic and explicit.** No ML-driven "choose the best text" — we pick summary OR raw, never both, never a heuristic blend.
6. **Truncate at 10 MB, don't reject.** Losing the tail of a giant payload is better than losing the whole document for retrieval.

## Open questions

1. **Cross-index search.** Do we expose a multi-index search endpoint in Cortex, or does the orchestrator (spec 11) issue N parallel single-index searches? Leaning orchestrator-side federation for control over ranking; revisit when spec 11 benchmarks land.
2. **Language-aware tokenization for code.** Meilisearch's tokenizer is natural-language tuned. Snake_case symbols may underperform. Custom tokenizer (or pre-processing pass) — defer until retrieval quality pass in Phase 2.

## References

- Architecture §5.2, §5.3 (retrieval lanes), §12 OQ on Lexum.
- Spec 01 — Event schema.
- Spec 02 — Storage layout (Meilisearch instance, index namespaces).
- Spec 04 — Cortex Core.
- Spec 05 — Classifier (`summary` usage).
- Spec 06 — Embedder (parallel consumer; same collection boundaries).
- Spec 07 — Graph writer (parallel consumer).
- Spec 11 — Query API (reads these indexes).
- Meilisearch docs: https://www.meilisearch.com/docs — settings API, typo tolerance, filterable/sortable attributes.
