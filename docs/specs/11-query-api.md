# 11 — Query API (hybrid retrieval + RRF)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 06, 07, 08

## Goal

Expose a single HTTP + MCP endpoint that orchestrates **vector + keyword + graph** retrieval across Vectorizer, Meilisearch, and Nexus, fuses results with **Reciprocal Rank Fusion**, and returns a structured context bundle ready to be injected into an AI agent's system prompt. Three callers: the Claude Code adapter (pre-thinking), the dashboard, and deep-analysis workflows. Latency is non-negotiable.

## Scope

**In:**
- `cortex-api` crate: Rust + Axum HTTP service + MCP tool bindings.
- Orchestrator: parallel fan-out, RRF fusion, decision/law overlay.
- Intent → retrieval-strategy mapping (`pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `free_search`).
- Result cache (Synap) keyed by semantic hash of `(intent, scope, query)`.
- Response envelope + per-field budgets (snippets, decisions, graph neighbors).
- Derived `SIMILAR_TO` edges (on-demand KNN, not materialized in graph).
- Telemetry.

**Out:**
- Pre-thinking-specific injection logic (spec 12 wraps this API with adapter-side heuristics).
- Law evaluation (spec 13 detector contract; `law_check` intent only retrieves, does not evaluate).
- Analysis-workflow orchestration (spec 15).
- Write path (owned by the worker specs 05–08).
- Query-log mining / A-B ranking — future.

## Inputs / Outputs

### HTTP endpoint

```
POST /v1/query
Content-Type: application/json
```

### Request schema

```jsonc
{
  "intent": "pre_change_context",         // required
  "scope": {
    "repo": "vectorizer",                  // optional, canonical lowercase (phase10d)
    "files": ["src/index/hnsw/*"],         // optional
    "topics": ["hnsw", "retrieval"],       // optional
    "since": "2025-01-01"                  // optional ISO8601
  },
  "query": "How do we tune ef_search for 1M-vector collections?",   // required
  "limit": 20,                             // default 20, max 100 (per field below)
  "k": 50,                                 // KNN top-k (vector lane); default 50, max 200
  "include": ["snippets", "decisions", "violations", "graph_neighbors", "similar_turns"],
  "budget_ms": 500,                        // total timeout; default 500
  "budget_bytes": 32768                    // phase11c — serialised JSON cap; default 32 KiB
}
```

`budget_bytes` (phase11c) bounds the size of the serialised
[`QueryResponse`](#response-shape) so the wire payload stays under the
MCP transport's per-tool-result cap. The daemon-side clipper trims
snippet text to ≤ 1 KiB each, then pops tail entries from
`graph_neighbors` → `similar_turns` → `violations` → `decisions` →
`snippets` until the byte budget fits. Removed counts are surfaced on
`response.clipped` so callers can branch on "more available" without a
follow-up query.

`scope.repo` is **canonical lowercase**. Mixed-case input (`"Cortex"`,
`"Vectorizer"`) is accepted for backwards-compat — the orchestrator
calls `to_ascii_lowercase` before forwarding to the lane filter so
`scope.repo: "Cortex"` and `scope.repo: "cortex"` resolve to the
same rows. See
[spec 02 §Naming canonicalization](./02-storage-layout.md#naming-canonicalization-phase10d)
for the cross-surface contract.

#### Scope filter contract (phase10h)

Pre-phase10h only `scope.repo` actually filtered; `scope.since`,
`scope.topics`, and `scope.files` were accepted by the
deserialiser but silently dropped at fan-out. Phase10h wires
every dimension end-to-end:

- **Meili keyword lane** — `meili_lane.rs::build_meili_filter`
  composes a per-index AND-joined expression. Capabilities are
  per-schema (`crates/cortex-storage/schemas/meili/*.settings.v1.json`):
  - Per-repo legacy `cortex-{slug}-{family}` indexes use `ts`
    for `since`, plus `repo` / `path` / `topics`.
  - Global `cortex_turns` / `cortex_decisions` /
    `cortex_analyses` use `occurred_at` for `since`, plus
    `repo` / `topics`.
  - `cortex_laws` carries no scope facets — clauses for that
    index are skipped silently rather than triggering Meili's
    "attribute not filterable" 4xx.
  - `cortex_knowledge` / `cortex_learnings` use `occurred_at`
    + `repo`.
- **Vectorizer lane** — the SDK's `search_vectors` does not
  expose a server-side filter parameter, so the lane filters
  client-side after metadata projection
  (`vectorizer_lane.rs::scope_matches`). The post-projection
  filter is **fail-open**: hits whose metadata lacks the
  field a scope dimension references round-trip rather than
  being dropped silently.
- **Nexus graph lane** — graph hits don't carry per-row
  timestamp / topic / path metadata in the projected
  `LaneHit`; scope filtering against the structural graph is
  deferred to a follow-up phase. Document scope dimensions
  still flow into `scope_resolved` so the audit envelope
  records what the caller asked for.
- **Audit envelope** —
  `cortex.events.query_audit.scope_resolved` carries the
  full canonicalised scope: lowercased `repo`, deduplicated
  + lowercased `topics`, trimmed `since` (RFC-3339), and
  trimmed non-empty `files`. `service.rs::canonicalise_scope`
  is the source of truth.

### Response schema

```jsonc
{
  "intent": "pre_change_context",
  "query_id": "01HY...",                   // ULID; echoed in logs / dashboard
  "scope_resolved": { /* canonicalized scope */ },
  "results": {
    "snippets": [
      {
        "rank": 1,
        "source": "vector",                // vector | keyword | graph
        "collection": "cortex-code",
        "repo": "Vectorizer", "path": "src/index/hnsw/mod.rs",
        "symbol": "hnsw_search",
        "content_hash": "sha256:...",
        "text": "...",
        "score": 0.78,
        "why": "vector match to 'ef_search tuning'"
      }
    ],
    "decisions": [
      { "rank": 1, "id": "DEC-0042", "title": "Raise HNSW ef_search default to 128",
        "status": "accepted", "ts": 1712000000000, "score": 0.82,
        "links": ["file://docs/decisions/0042-hnsw-ef-default.md"] }
    ],
    "violations": [
      { "id": "LV-...", "law_id": "LAW-012", "severity": "notable",
        "message": "HNSW recall benchmarks must run before merge",
        "observed_in": "turn:01HXZ..." }
    ],
    "graph_neighbors": [
      { "from": "ToolCall:01H...", "relation": "TOUCHED",
        "to": "Artifact:Vectorizer|src/index/hnsw/mod.rs|sha256:...",
        "hops": 1 }
    ],
    "similar_turns": [
      { "turn_id": "01HXZ...", "ts": 1710000000000, "model": "claude-sonnet",
        "summary": "Refactored ef_search default...", "score": 0.74 }
    ]
  },
  "laws_active": [ { "id": "LAW-012", "severity": "notable", "title": "..." } ],
  "budget": { "used_ms": 142, "cap_ms": 500, "cache": "miss" },
  "debug": { "lanes": { "vector_ms": 48, "keyword_ms": 33, "graph_ms": 71 } },
  // Optional. Present when the orchestrator wants to flag a structural
  // condition the caller would otherwise miss — today the only emitter
  // is `repo_not_indexed`, fired when `scope_resolved.repo` does not
  // appear in the daemon's keyword-lane snapshot (the same set
  // `/v1/status.indexed_repos` reports).
  "notice": {
    "code": "repo_not_indexed",
    "message": "scope.repo `<slug>` is not present in the cortex-api indexed-repo snapshot",
    "hint": "run `cortex-bootstrap --repo <path>` to seed the daemon for this repo, then retry."
  },
  // Phase11c — populated when the byte-budget clipper trimmed the
  // response. Absent when nothing was removed.
  "clipped": {
    "removed_snippets": 12,
    "removed_decisions": 0,
    "removed_violations": 0,
    "removed_similar_turns": 0,
    "removed_graph_neighbors": 0,
    "snippets_text_clipped": 5,
    "final_bytes": 31872,
    "budget_bytes": 32768
  }
}
```

Fields inside `results` are only present when requested via `include`. Missing-by-request is different from missing-by-error (the latter surfaces in `debug.errors`). The optional top-level `notice` is omitted from the wire when `null` (`skip_serializing_if = Option::is_none`).

### MCP tool binding

Identical schema, exposed as an MCP tool (`cortex.query`) so agent hosts can call it without speaking HTTP.

## Design

### Intent → strategy table

| Intent                 | Vector | Keyword | Graph expansion                                     | Overlay                          |
|------------------------|:------:|:-------:|-----------------------------------------------------|----------------------------------|
| `pre_change_context`   | ✅     | ✅      | `Artifact -[:TOUCHED]-> ToolCall -[:HAS_TOOL_CALL]-> Turn -[:LINKED_TO]-> Decision`, 1–2 hops | decisions + active laws in scope |
| `decision_lookup`      | ✅     | ✅      | Decision → supersession chain                        | —                                |
| `similar_problems`     | ✅     | —       | Turn → Analysis → Decision                            | —                                |
| `law_check`            | —      | ✅      | `Law -[:OF]-> LawViolation -[:OBSERVED_IN]-> Turn` (last 30d) | active laws + violations         |
| `free_search`          | ✅     | ✅      | none                                                 | —                                |
| `explain`              | ✅     | ✅      | none (until phase4c lands `edge_artifact_definitions`) | —                                |

Strategies live in `cortex-api/src/strategies.rs`, one `fn` per intent, return an execution plan the orchestrator runs.

#### Lane composition per intent (phase10a)

The 2026-04-29 relevance audit caught three intents fanning out to
per-repo collections / indexes that frequently carried no data,
producing 0% recall on `law_check` and `similar_problems` and 50%
on `decision_lookup` (matched only when the prompt happened to hit
a decision title). Phase10a routes those intents to the **global**
stores defined in `crates/cortex-storage/src/names.rs`:

| Intent             | Vector collection                                          | Keyword index               | Graph template                     |
|--------------------|------------------------------------------------------------|-----------------------------|------------------------------------|
| `pre_change_context` | `cortex-{slug}-code` + `cortex-{slug}-docs` (per-repo)   | `cortex-{slug}-{code,docs,decisions}` | `edge_artifact_touched_neighbours` |
| `decision_lookup`  | `cortex.decision.fp32` (global)                            | `cortex_decisions` (global) | `decision_supersedes_chain`        |
| `similar_problems` | `cortex.turn.fp32` + `cortex.turn.pq` (global)             | `cortex_turns` (global)     | `turn_analysis_decision_chain`     |
| `law_check`        | —                                                          | `cortex_laws` (global)      | `law_violations_last_30d`          |
| `free_search`      | `cortex-{slug}-code` (per-repo)                            | `cortex-{slug}-code`        | —                                  |
| `explain`          | `cortex-{slug}-code` + `cortex-{slug}-docs` (per-repo)     | `cortex-{slug}-{code,docs}` | —                                  |

For the global indexes that carry `repo` as a filterable attribute
(`cortex_turns`, `cortex_decisions`), the keyword lane translates
`scope.repo` into a `repo = '<slug>'` Meili filter so a repo-scoped
query does not bleed across other repos sharing the same global
index. `cortex_laws` does NOT carry `repo` — laws are cross-repo by
design and `law_check` strips `scope.repo` before fan-out.

### Execution plan

```rust
enum Lane { Vector { collection: String, k: usize }, Keyword { index: String }, Graph { cypher: String, params: Value } }

struct Plan {
    lanes: Vec<Lane>,        // run in parallel
    overlays: Vec<Overlay>,  // post-fusion enrichment
    fuse: FusionStrategy,    // RRF by default
}
```

### Fan-out + fusion

- Lanes execute in parallel with `tokio::join_all`. Each lane carries its own sub-budget (the orchestrator splits the total `budget_ms` with a 40/40/20 default: vector / keyword / graph).
- **Score-aware Reciprocal Rank Fusion** (phase6c — extends Cormack et al. 2009 with the lane-native score the lanes already capture into `LaneHit.score`):

  ```text
  fused(d) = Σ_lanes [ alpha * (1 / (k + rank_lane(d)))
                     + (1 - alpha) * lane.normalized_score(d) ]
  ```

  - `alpha = 1.0` reproduces the pre-phase6c positional-only RRF byte-for-byte (regression escape hatch).
  - `alpha = 0.0` ranks by summed normalised native score alone.
  - Default `alpha = 0.7` biases toward RRF stability while letting strong lane-native scores break weak-positional ties so a single weak graph hit at rank 1 doesn't outrank a dense vector top-3.
  - `k` defaults to 60 (Cormack et al.). Larger `k` flattens the per-lane curve; smaller `k` emphasises rank-1 hits.
- Tie-breaks: prefer recency (higher `ts`) then `severity` (critical > notable > info), then `doc_id` for determinism.
- Max per-field output is `limit` (response field; default 20). If fusion yields more, the tail is dropped.

#### Per-lane normalised score convention

Each lane's `LaneHit.score` is mapped onto `[0.0, 1.0]` by `LaneHit::normalized_score()` (NaN / infinity collapse to `0.0`). Lanes that already produce `[0,1]`-valued scores round-trip unchanged:

| Lane | Native score | Normalisation |
|------|--------------|---------------|
| Vectorizer | cosine similarity, `[0, 1]` | identity |
| Meili keyword | `_rankingScore`, `[0, 1]` | identity |
| Nexus graph | currently `0.0` (no contribution) | identity until `phase4c` lands path-length-derived scoring (`1.0` direct neighbour → `0.5` 2-hop → `0.25` 3-hop) |

#### Tuning knobs

| Env var | Type | Default | Range |
|---------|------|---------|-------|
| `CORTEX_RRF_ALPHA` | `f32` | `0.7` | `[0.0, 1.0]` (clamped; out-of-range logs WARN and falls back to default) |
| `CORTEX_RRF_K` | `u32` | `60` | `>= 1` (clamped; `0` logs WARN and falls back to default) |

The resolved `(alpha, k)` lands on every audit envelope as `fusion_alpha` / `fusion_k` so phase6e's recall/MRR harness can attribute regressions to fusion-tuning changes without re-running the queries. Closes [F-005 in `docs/analysis/relevance/01-findings.md`](../analysis/relevance/01-findings.md).

### Overlays

After fusion, the orchestrator annotates results:

- **Decisions overlay:** for any result in the scope, attach `LINKED_TO` Decisions (1 hop).
- **Laws overlay:** attach active laws matching scope.repo + scope.topics.
- **Graph neighbors:** optional (only if `include=graph_neighbors`), runs a single Cypher query expanding top-5 fused results 1 hop.

### Similar turns (`similar_turns`)

Derived edge: given seed nodes from fusion, run KNN in Vectorizer's `cortex-turns` collection with the seed's embedding. Top-5 are returned with `SIMILAR_TO.score`. **Not persisted** — spec 07 §Decisions §3.

### Snippet `text` field — kind-aware projection (phase6g)

`Snippet.text` is the projected document body the orchestrator
hands back to callers. The keyword lane
([`crates/cortex-api/src/meili_lane.rs`](../../crates/cortex-api/src/meili_lane.rs))
picks the source field with a kind-aware precedence chain — see
[spec 08 §Read-side projection](./08-fulltext-indexer.md#read-side-projection-phase6g)
for the table. The short version: artifact / law_violation hits
project from `body` first (so the actual file content / violation
message reaches the bundle); curated kinds (decision / analysis /
memory) keep `summary > title > body`; turn-shaped kinds use
`summary > body > title`. `Snippet.path` is populated separately,
so the operator-visible context (file path, repo) is preserved
even when `text` switches to body content. Closes
[F-009](../analysis/relevance/01-findings.md).

#### phase10b — body capture + path/text separation

The 2026-04-29 audit caught snippets where `text` carried the
*path* and `symbol` carried the *kind label* (`artifact`, `turn`,
…), producing pre-thinking bundle rows like
`Cortex/.../types.rs:artifact — \n   crates/cortex-api/src/types.rs`
— an `ls`-grade result the agent could not act on. Phase10b
hardens both fields:

- **Snippet.text** never echoes the path back. When every slot in
  the projection chain holds the same string as `Snippet.path`
  (typical for body-less artifact docs whose `derive_title` set
  `title = path`), the lane returns empty `text` instead of
  surfacing the path as content.
- **Snippet.body_truncated** (new, additive boolean field) is
  stamped to `true` when the lane could not project a body
  distinct from the path. The pre-thinking renderer reads this
  to render the header alone with a `(body not indexed inline)`
  cue instead of an empty body block.
- **Snippet.symbol** never carries event-kind labels. The
  orchestrator's `snippet_from_hit` filters
  `artifact / turn / tool_call / agent_call / decision /
   analysis / memory / law_violation` out of `LaneHit.symbol`
  before it reaches the wire; decisions surface their
  `decision_title` extras (phase10a) as the symbol so bundle
  headers carry the real ADR title instead of the literal string
  `decision`.

Wire-shape change: `body_truncated: bool` defaults to `false` and
skip-serialises, so existing consumers keep working unchanged.

### Lane projection contract (phase6b)

Every overlay derivation reads its inputs out of `LaneHit.extras`. Live lane impls (`MeiliKeywordLane`, `VectorizerLane`) MUST stamp the contract keys below into `extras` whenever the upstream document carries them; missing keys round-trip as absent and the overlay deriver skips that row.

The constant `cortex_api::lanes::LANE_EXTRAS_KEYS` is the source of truth and the regression guard at `crates/cortex-api/src/lane_contract.rs` pins it.

| Key | Upstream source | Consumed by |
|-----|-----------------|-------------|
| `decision_id` | Meili top-level `decision_id` (or `_meta.decision_id` during fulltext-worker rollouts) / Vectorizer `metadata.decision_id` (or `metadata.payload.decision_id` for legacy embedder builds) | `derive_decisions` |
| `decision_status` | upstream `status` for decision rows | decisions overlay status badge |
| `supersedes` | upstream `supersedes[]` array | decision detail link chain |
| `turn_id` | upstream `turn_id` | `derive_similar_turns` |
| `model` | upstream `model` (turn rows) | similar_turns overlay |
| `summary` | upstream `summary` (turn rows; also used as snippet text fallback when `body` is empty) | similar_turns overlay |
| `law_id` | upstream `law_id` | `derive_laws` |
| `severity` | upstream `severity` (also projected onto the top-level `LaneHit.severity` field for the violations overlay tie-break) | `derive_laws` / violations overlay |

Lookup precedence in each lane:

- **Meili keyword lane**: `_meta.<key>` (canonical post-migration nesting) → top-level `<key>` → typed slot (only for `summary` / `severity`, which `MeiliDoc` parses into a named field).
- **Vectorizer lane**: `metadata.<key>` (current SDK ≥ 3.0.3) → `metadata.payload.<key>` (legacy embedder-worker shape).

A `kind = "decision"` document landing without a `decision_id` is a worker-side projection bug: the keyword lane emits `tracing::debug!` when this happens so the gap is visible without flooding production at INFO/WARN.

Closes [F-007 in `docs/analysis/relevance/01-findings.md`](../analysis/relevance/01-findings.md).

### Caching

- **Key:** `hash(intent || scope || embed(query) || schema_version)`.
- **Store:** Synap KV, TTL 10 min.
- **Granularity:** whole response, not per-lane. Simpler invalidation.
- **Invalidation:** any ingestion event tagged `severity=critical` within `scope.repo` flushes matching cache entries (published to Synap `cortex.cache.invalidate`).
- **Hit response:** returns cached bundle with `budget.cache = "hit"` and skips all lane calls.

Embedding the query for the cache key is expensive; for hot scopes we additionally cache the **query embedding** for 1 min so re-issued queries hit a sub-millisecond path.

### Pagination

- Not exposed in v1. `limit` is a hard cap; callers needing deeper lists issue follow-up queries with a narrower `scope`.
- Justification: hot-path callers (adapters) have tight bundle budgets (32 KB, spec 10) that make pagination irrelevant. Analysis workflows (spec 15) drive their own retrieval loops.

### Rate limiting

Per-caller token bucket (`cortex-adapter-claude` / `dashboard` / `analysis` / other). Defaults: 30 rps sustained, 60 rps burst per caller. 429 responses include `Retry-After`.

### Security / privacy

- The query API re-validates scope against per-caller ACLs (`cortex-api` owns a small ACL store: `caller` → allowed `repo` list).
- Response bodies pass through a final redaction pass (belt-and-suspenders: storage writers already redacted; the reader re-redacts using the same pattern catalog).
- Audit: every request is logged to `cortex.events.query_audit` (own Synap stream) with `caller`, `query_id`, `intent`, `scope`, result counts, `latency_ms`.

#### `GET /v1/audit/{query_id}` (phase10j)

Synchronous read of the audit envelope for a previous request. Backed
by an in-memory ring buffer (`audit_store`, default capacity 1024)
that records every envelope alongside the synap publish. Used by the
[`cortex_audit` MCP tool](18-claude-code-plugin.md#cortex_audit) so an
agent can debug zero-hit retrievals without leaving the MCP transport.

Response body — the canonical `query_audit` envelope:

```json
{
  "kind": "query_audit",
  "caller": "claude-code-plugin",
  "query_id": "01KQDX...",
  "intent": "free_search",
  "scope": { "repo": "cortex" },
  "scope_resolution": "explicit",
  "counts": { "snippets": 0, "decisions": 0, "violations": 0,
              "graph_neighbors": 0, "similar_turns": 0, "laws_active": 0 },
  "latency_ms": 12,
  "cache": "miss",
  "fusion_alpha": 0.7,
  "fusion_k": 60,
  "intent_trigger": null,
  "query_rewrite_strategy": "passthrough",
  "vector_query": "...",
  "keyword_query": "..."
}
```

Failure modes:

| Status | Reason                       | Cause                                                  |
|--------|------------------------------|--------------------------------------------------------|
| 400    | `query_id_required`          | Empty path segment                                     |
| 404    | `audit_envelope_not_found`   | Daemon restarted, envelope aged out, or wrong replica |

The store is per-process and FIFO-bounded — a 404 is not authoritative
"this query never ran"; it is "this daemon does not currently have
the envelope retained". Long-term retention lives on the synap stream
+ the dashboard's timeline.

### Failure modes

| Failure                                    | Handling                                                             |
|--------------------------------------------|----------------------------------------------------------------------|
| Vectorizer unreachable                     | Lane returns empty; `debug.errors.vector` set; fusion continues       |
| Meilisearch unreachable                    | Lane returns empty; `debug.errors.keyword` set                        |
| Nexus unreachable                          | Lane returns empty; overlays skipped; `debug.errors.graph` set        |
| Lane exceeds sub-budget                    | Soft-cancel; partial ranks kept if any; `debug.lanes.<lane>.partial` |
| Total budget exceeded                      | 200 response with whatever's ready; `debug.truncated = true`          |
| Cache corruption / deserialization error    | Ignore cache, run lanes, overwrite entry                              |
| Empty query                                 | 400 `reason=empty_query`                                              |
| ACL deny                                    | 403 `reason=scope_forbidden`                                          |
| Scope unresolved (phase6a / F-003)          | 422 `reason=scope_repo_required` — see "Scope resolution" below       |

### Scope resolution (phase6a)

`POST /v1/query` MUST resolve `scope.repo` before running the orchestrator. The handler walks the following lanes in order; the first hit wins, and the chosen lane is recorded on the audit envelope as `scope_resolution`:

1. **`request.scope.repo` (explicit)** — round-trip unchanged. Audit value: `explicit`.
2. **`x-cortex-repo` header** — set by the MCP server (when the tool's input carries an explicit scope), the dashboard sidebar (when the user has a single repo filter active), or any other authenticated caller. Audit value: `header`.
3. **`x-cortex-cwd` header** — caller hint. The handler runs `cortex_storage::names::slug_for_repo(basename(cwd))` and stamps the slug on `request.scope.repo`. Audit value: `cwd`.
4. **Reject** — every lane missed. Return `422 { "reason": "scope_repo_required" }`. The previous fallback that routed to the `cortex-unknown-{family}` slug is removed; that slug is empty across all backends and the silent zero-hit response was the largest single relevance gap in the audit (F-003).

A `CORTEX_ALLOW_UNKNOWN_SCOPE=1` escape hatch keeps the legacy fallback alive for one deprecation window: when set, the handler logs `tracing::warn!` and accepts the empty scope with audit value `rejected_legacy`. The hatch is removed at the harness gate (`phase6e`) once the audit shows zero remaining callers.

MCP `cortex_query` injects `x-cortex-cwd` from the operator's working directory; the dashboard injects `x-cortex-repo` from the active sidebar filter; the pre-thinking pipeline keeps populating `request.scope.repo` directly via `scope::derive`. See [`docs/specs/18-claude-code-plugin.md`](18-claude-code-plugin.md) for the MCP-side header contract.

### Observability

```
cortex.api.query.requests.total       counter, labels: intent, caller, cache
cortex.api.query.latency_ms           histogram, labels: intent, cache
cortex.api.query.lane.latency_ms      histogram, labels: lane
cortex.api.query.lane.errors          counter, labels: lane, status
cortex.api.query.results.count        histogram, labels: intent, field
cortex.api.query.rate_limit.drops     counter, labels: caller
cortex.api.query.cache.hits / .misses counter
```

## Acceptance criteria

- [ ] `POST /v1/query intent=pre_change_context` against a bootstrapped Vectorizer returns ≥1 snippet, ≥0 decisions, ≥0 violations within 500 ms budget.
- [ ] Cache: second identical request returns with `budget.cache = hit` in ≤20 ms.
- [ ] Fan-out parallelism: artificial 200 ms delay on Nexus does not add 200 ms to the end-to-end latency when the 300 ms budget allows the other two lanes to finish in parallel.
- [ ] RRF correctness: on a golden set of 50 hand-labeled queries, top-5 precision ≥ 0.7 (baseline; we improve in Phase 2 retrieval-quality passes).
- [ ] Budget enforcement: setting `budget_ms=100` returns partial results with `debug.truncated=true`; no stragglers are awaited.
- [ ] Overlays: an in-scope `Decision` is attached to the top result via `results.decisions`.
- [ ] Similar-turns derivation: KNN against `cortex-turns` returns 5 entries ordered by score.
- [ ] Law check intent: returns only the `violations` field (no snippets) by default, even if `include` is the default set.
- [ ] MCP binding: `cortex.query` tool exposed; an MCP client can invoke it with the same payload.
- [ ] Caller ACL: a caller without `Vectorizer` in its allowed list gets a 403 on `scope.repo = Vectorizer`.
- [ ] Rate limiter: bursts beyond 60 rps get 429 with `Retry-After`; sustained load at 30 rps passes.
- [ ] Redaction pass: a synthetic embedded secret in a chunk's `text` is replaced with `[REDACTED]` in the response.
- [ ] Lane failure: Meilisearch off → `debug.errors.keyword` present, other lanes still return results, 200 response.
- [ ] Cache invalidation: a severity=critical ingestion event invalidates matching cache entries within 2 s; subsequent identical query gets `cache = miss`.
- [ ] P50 < 50 ms, P95 < 150 ms (cached); cold P95 < 500 ms on a pre-warmed dev stack.
- [ ] Query audit stream carries one event per request.

## Decisions

1. **RRF over learned-to-rank.** RRF is parameter-free, robust, and cheap. Learning-to-rank is deferred to Phase 2 once we have enough query-log data.
2. **Whole-response caching, not per-lane.** Simpler; invalidation is coarse but correct. Per-lane cache would need per-lane invalidation — not worth it at this scale.
3. **`SIMILAR_TO` is never stored.** Re-derive per query; embeddings drift and stored similarities rot.
4. **Scope is a filter, not a hint.** Out-of-scope results are dropped, not down-ranked. This is what makes pre-thinking context safe to inject.
5. **Final redaction pass at read time.** Defense in depth — the storage writers are authoritative, but one bad write shouldn't leak forever.
6. **No pagination in v1.** Hot-path callers want small bundles; analysis callers drive their own loops.
7. **Cache key includes a `schema_version`.** Any migration-worthy change to the event or graph schema invalidates every cached bundle on deploy.

## Relevance harness (phase6e)

The canonical relevance gate for this API is the
[`cortex-relevance-eval`](../../crates/cortex-relevance-eval) binary,
backed by the labeled query set under
[`tests/relevance/queries.toml`](../../tests/relevance/queries.toml)
(≥10 entries per intent across all five intents). Closes
[F-008](../analysis/relevance/01-findings.md#f-008--relevance-is-unmeasured-there-is-no-labeled-query-set-no-recallk--mrr-harness-no-canary-queries-that-catch-regressions).

### Metrics

For each labeled query, the harness POSTs to `/v1/query` and inspects
the top-10 fused snippets:

- `recall_at_10` — boolean: did **any** `expected_doc_ids` entry
  appear in the top-10 snippets? Matches the canonical composite id
  (`<repo>|<path>|<content_hash>`), any individual snippet field
  (`repo`, `path`, `symbol`, `content_hash`, `collection`), or a
  substring of `path` / `symbol`.
- `mrr` — `1.0 / rank_of_first_match` for hits, `0.0` for misses.

Per-intent buckets and a global aggregate are emitted as
`recall_at_10_pct` (`matches / total * 100.0`) and `mrr_avg`.

### Report shape

Each run pretty-prints to `target/relevance/<git-sha>.json`:

```jsonc
{
  "generated_at": "2026-04-29T12:34:56Z",
  "git_sha": "abc123",
  "api_version": "0.1.0",
  "omitted_intents": [],
  "per_intent": {
    "explain":            { "total": 10, "matches": 8, "recall_at_10_pct": 80.0, "mrr_avg": 0.71 },
    "pre_change_context": { "total": 10, "matches": 7, "recall_at_10_pct": 70.0, "mrr_avg": 0.62 }
    // …one bucket per intent
  },
  "global": { "total": 50, "matches": 38, "recall_at_10_pct": 76.0, "mrr_avg": 0.66 },
  "queries": [
    { "id": "rel-001", "intent": "pre_change_context",
      "query": "...", "recall_at_10": true, "matched_rank": 2,
      "mrr": 0.5, "matched_doc_id": "crates/.../strategies.rs",
      "returned": 10 }
    // …one row per query, sorted by id
  ]
}
```

### Regression gate

CI ([`.github/workflows/relevance.yaml`](../../.github/workflows/relevance.yaml))
runs the harness on every PR touching a retrieval-path file and on
every push to `main`. Each PR run is compared against the cached
`main` baseline:

- **Hard gate** — global `recall_at_10_pct` and `mrr_avg * 100` must
  stay within `2pp` of baseline. The harness exits `2` on a hard
  regression and the CI step fails.
- **Soft gate** — per-intent metrics within the same `2pp` band emit
  warnings (`soft_regressions: [...]`) but do not fail the run.
- **Worst-5 surfacing** — the verdict ships the five most-regressed
  query ids so triagers can re-create the failure locally with
  `--query-set tests/relevance/queries.toml --baseline ...`.

When `/v1/status` is unreachable, or a fixture's `scope.repo` is not
in the daemon's `indexed_repos` snapshot, the affected intent buckets
land in `omitted_intents` and the harness keeps going — partial
visibility beats a hard failure that hides every other regression.

### Persistence

On merge to `main`, the workflow copies the report to
`.rulebook/learnings/relevance/<YYYY-MM-DD>-<sha>.json` so the
dashboard's relevance trend view (spec 16) can render the time series
without re-running CI. Identical-content reports are skipped to keep
the history readable.

## Query rewriting (phase6f)

A pre-pass between intent selection and lane fan-out rewrites the
free-form prompt into queries optimised for each lane. Closes
[F-004](../analysis/relevance/01-findings.md). The
[`QueryRewriter`](../../crates/cortex-api/src/query_rewrite.rs)
trait has three shipped implementations:

| Strategy | When | What it does |
|---|---|---|
| `passthrough` | kill-switch / A/B baseline | Copies the prompt verbatim into both lanes; reproduces the pre-phase6f behaviour. |
| `noun_phrase` | **default** | Deterministic, no LLM. Strips leading question words (`why`/`how`/`what`/...), drops a curated stop-list, keeps tokens that look like identifiers (camelCase / snake_case / kebab-case / paths / file extensions). Same string goes to both lanes. |
| `sonnet` | opt-in | Spawns the Claude Code CLI (`claude -p - --model <model> --output-format json`) per cache miss and parses the JSON envelope; produces distinct `vector_query` and `keyword_query`. Cached by `sha256(prompt + intent)` for 24h. On timeout / missing binary / non-zero exit / malformed JSON, transparently falls back to `noun_phrase` and stamps the audit envelope as `sonnet_fallback_noun_phrase` so operators can tell the two paths apart. **No Anthropic API key required** — same CLI pattern as `cortex-classifier` and `cortex-api/src/analyzer.rs::invoke_cli`. |

### Selection

Set `CORTEX_QUERY_REWRITER` to `noun_phrase` (default), `sonnet`,
or `passthrough`. Unknown values log a `WARN` and fall back to
`noun_phrase`. The boot log records the resolved strategy:

```
INFO query rewriter resolved (CORTEX_QUERY_REWRITER) rewriter=noun_phrase
```

The `sonnet` path honours `CLAUDE_CODE_BIN` (path to the
`claude` binary; default resolved against `PATH`),
`CORTEX_REWRITER_MODEL` (default `claude-sonnet-4-6`), and
`CORTEX_REWRITER_TIMEOUT_MS` (default `1500`). When the binary is
missing or fails to spawn, every Sonnet call lands on the
fallback path and the rewriter returns the deterministic
`noun_phrase` strip — deployments that opt in without the CLI
installed still get useful rewrites rather than failing user
requests. The Cortex stack never uses the Anthropic HTTP API
directly; this matches `cortex-classifier` and the analyzer's
`invoke_cli` path so operations only need the CLI on `PATH`.

### Audit envelope additions

Every audit envelope now stamps three new fields:

- `query_rewrite_strategy` — `"passthrough"` / `"noun_phrase"` /
  `"sonnet"` / `"sonnet_fallback_noun_phrase"`
- `vector_query` — what landed on the vector lane
- `keyword_query` — what landed on the keyword lane

The phase6e harness uses these to attribute uplift to the
rewriter; operators read them when a query routes somewhere
unexpected.

### Pipeline order

```
HTTP /v1/query
  → resolve_scope (phase6a)
  → intent_select (phase6d)
  → cache lookup
    └─ miss → Orchestrator::run
         ├─ rewriter.rewrite(prompt, intent)   ← phase6f
         ├─ build_plan(req)
         ├─ patch plan.{vectors,keywords,graphs}.query with rewritten output
         └─ fan-out + RRF (phase6c) + overlays
  → cache.put + audit.publish (with rewrite + fusion + scope context)
```

### Decision rule for shipping `sonnet`

The phase6e harness is re-run against `passthrough`, `noun_phrase`,
and `sonnet`. `sonnet` becomes the default ONLY if its global
`recall@10` beats `noun_phrase` by ≥3pp; otherwise `noun_phrase`
stays the default and `sonnet` remains opt-in. The decision is
documented in `.rulebook/learnings/relevance/<date>-rewriter-decision.md`.

## Open questions

1. **Per-intent RRF weighting.** Does `decision_lookup` want to weight keyword higher than vector? Defer to first quality pass.
2. **Federated multi-tenant scope.** HivehubCloud story (Phase 5) may need per-tenant orchestrators. Out of scope here; the API surface must not preclude it.

## References

- Architecture §5.3 (retrieval), §8 (end-to-end), §11 Phase 1 (quality targets).
- Spec 01 — Event schema.
- Spec 02 — Storage layout (collections/indexes).
- Spec 06 — Embedder (vector lane source).
- Spec 07 — Graph writer (graph lane source).
- Spec 08 — Full-text indexer (keyword lane source).
- Spec 10 — Claude Code adapter (primary latency-sensitive caller).
- Spec 12 — Pre-thinking injection (consumes this API; defines adapter-side heuristics).
- Spec 13 — Laws DSL (law data source for the overlay).
- Spec 14 — Governance engine (`violations` source).
- Spec 15 — Deep Analysis (heavy caller; drives its own retrieval loops).
- Reciprocal Rank Fusion: Cormack, Clarke, Büttcher — SIGIR 2009.
