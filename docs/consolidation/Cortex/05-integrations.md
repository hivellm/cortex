# Cortex — Service Integrations

## Dependency Matrix

Cortex orchestrates five external HiveLLM services + one external open-source service, all containerized in `docker-compose.yml`:

| Service | Image | Port | SDK Version | Role | Cortex Crate | Health Check |
|---------|-------|------|-------------|------|--------------|--------------|
| **Vectorizer** | `hivehub/vectorizer:3.3.0` | 17001 | vectorizer-sdk 3.3.0 | Dense embedding storage; semantic search | cortex-embedder-worker | ✅ `/health` (implicit) |
| **Nexus** | `hivehub/nexus:2.2.0` | 17002 | nexus-graph-sdk 2.1 | Graph storage; Cypher traversal | cortex-graph-worker | ✅ `/health` (explicit healthcheck) |
| **Synap** | `hivehub/synap:latest` | 17003/17013/16379 | synap-sdk 0.12 | Event bus; pub/sub; ingestion stream | cortex-workers (all) | ✅ `http://synap:15500/health` |
| **Meilisearch** | `getmeili/meilisearch:v1.10` | 17004 | (HTTP client) | Full-text inverted index; BM25 keyword search | cortex-fulltext-worker | ✅ `http://meilisearch:7700/health` |
| **Claude** | (CLI: Anthropic) | N/A | anthropic API or CLI 2.x | Classification (Haiku); analysis (Sonnet) | cortex-classifier-worker; cortex-api | ✅ Graceful fallback |
| **Rulebook** | (sister repo, MCP + SDK) | N/A | (library) | Laws/decisions/learnings federation | cortex-bootstrap, cortex-api | ✅ MCP tool availability |

## Per-Service Integration Detail

### Vectorizer (HivehubCloud, image `hivehub/vectorizer:3.3.0`)

**Health:** 🟡 — SDK drifts partially mitigated; vectors queryable downstream.

| Interaction | State | Notes |
|-------------|-------|-------|
| Auth (`/auth/login` → JWT) | ✅ Works | SDK 3.0.3 path works when env var contains actual JWT; direct login now supported (commit c41dab0) |
| `create_collection` / `get_info` | ✅ Works | Cortex uses per-repo collections (`cortex-{repo}-{family}`) |
| `insert_texts` (batch upsert) | ⚠️ Drifted | Server reports `total_failed=4-5` per 64-doc batch but `vector_count=0` in response; vectors are queryable despite this. [Anti-pattern recorded](../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md). 2 of 6 drifts resolved; 3, 4, 5, 6 still open server-side. |
| `search_vectors` | ✅ Works | Used by `VectorLane` in query API; returns ranked hits. Audit score: top-1 ≈ 0.136 (BM25-style, weak). |
| `list_vectors` / `get_vector` | ❌ Drifts | Server returns synthetic 200 for any ID; SDK has no list path. Workaround: `LiveVectorizerClient::list_stored_chunk_ids` via direct reqwest. [ADR 001](../../.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md) (superseded for login/insert; `get_vector` path still bypassed). |

**Action:** Post-upsert verification or SDK 3.0.4 fix. Phase4d diagnosis planned.

### Nexus (image `hivehub/nexus:2.2.0`)

**Health:** 🟡 — UNWIND-writes silently drop; per-row workaround in place.

| Interaction | State | Notes |
|-------------|-------|-------|
| `/health` | ✅ OK | Explicit healthcheck working |
| Single-statement Cypher writes | ✅ OK | Per-row renderers (commit 1e34417); `assert_write_landed` validates landing (commit 5bd0185) |
| `UNWIND` batch writes | ❌ Silently drop | Batch parameter substitution no-ops; [anti-pattern recorded](../../.rulebook/knowledge/anti-patterns/cypher-unwind-write-and-param-write-substitution-silently-drop-in-nexus-1-15.md). |
| Parameter substitution | ❌ Same as UNWIND | Param binding silently no-ops on writes |
| Driver reports success | ⚠️ Misleading | Reports `nodes_upserted=271` but `MATCH (n)` post-bootstrap returns 1 unlabeled node. Post-write assert now catches this. |

**Action:** Keep per-row path (safe if UNWIND fixes); integration test for `(:Symbol)-[:DEFINES]->(:Artifact)` once phase4c emits them (planned).

### Synap (image `hivehub/synap:latest`)

**Health:** 🟢 — Most reliable; auto-recovery pattern prevents bootstrap races.

| Interaction | State | Notes |
|-------------|-------|-------|
| Stream consume | ✅ OK | cortex-classifier-worker, cortex-embedder-worker, cortex-fulltext-worker, cortex-graph-worker all consume `cortex.events.enriched` |
| Stream publish | ✅ OK | cortex-ingestion publishes to `cortex.events.raw|bootstrap` |
| Room creation on first 404 | ✅ OK | Auto-create pattern (commit 1dc867f); prevents chicken-and-egg. [Pattern recorded](../../.rulebook/knowledge/patterns/synap-publisher-should-auto-create-rooms-on-first-not-found.md). |
| Pub/sub for dashboard SSE | ✅ OK | Live timeline stream uses Synap → cortex-api SSE bridge (commit ac10b5e) |
| Per-event publish tolerance | ✅ OK | 5% or 20-floor strategy (commit 845b5eb) accommodates first-time "Room not found" misses |

This is the most stable integration point.

### Meilisearch (image `getmeili/meilisearch:v1.10`)

**Health:** 🟡 — Single-repo coverage; worker offset issue suspected.

| Interaction | State | Notes |
|-------------|-------|-------|
| Index create / settings PATCH | ✅ OK | Once tooling-only fields stripped at client boundary (anti-pattern: [don't bake tooling-only fields into payloads sent verbatim to strict downstreams](../../.rulebook/knowledge/anti-patterns/don-t-bake-tooling-only-fields-into-json-payloads-sent-verbatim-to-a-strict-downstream.md)) |
| Document upsert with `primaryKey=id` | ✅ OK | Pinned in commit 99377a1 |
| Search | ✅ OK | `MeiliKeywordLane` returns results with score + source attribution |
| Per-project index naming | ✅ OK | `cortex-{repo}-{family}` enforced by [routing.rs:105-107](../../../crates/cortex-fulltext/src/routing.rs#L105-L107) |
| Cross-repo coverage | ❌ Fan-out gap | Only `cortex-cortex-*` populated despite 3+ repos walked. Worker offset/consumer state suspected. |
| Legacy indexes | 🔴 Hygiene | 6 empty indexes from pre-slug naming. Phase4a cleanup. |

**Action:** Diagnose worker consumer offset (phase4a); clean up legacy indexes.

### Claude (Haiku via CLI, Sonnet via CLI/API)

**Health:** 🟢 — Two fallback paths, graceful degradation.

| Interaction | State | Notes |
|-------------|-------|-------|
| Haiku CLI classification | 🟡 Mostly disabled | Command: `claude -p "$PROMPT" --model claude-haiku-4-5 --output-format json`. Works but **disabled in favor of StaticClassifier** (offline, zero cost, deterministic). CLI mode via `CORTEX_CLASSIFIER_MODE=cli` opt-in. |
| `--max-tokens` CLI flag | ✅ Fixed | Removed for Claude Code CLI 2.x compatibility (commit c41dab0) |
| Sonnet cross-event analysis | ✅ Added | Commit a62fcbd. Produces session summaries. Two paths: CLI (when on PATH) + direct Anthropic API (server/CI scenarios). |
| Fallback | ✅ Graceful | Analyzer config falls back if neither CLI nor API key available. Server scenarios (CI, container) use direct API. |

No Cortex-side drift; all working as designed.

### Rulebook (sister project, MCP + library)

**Health:** 🟢 — Federation working.

| Interaction | State | Notes |
|-------------|-------|-------|
| Task management (`mcp__rulebook__*`) | ✅ OK | Mandatory for task lifecycle per [CLAUDE.md](../../CLAUDE.md) |
| Memory (BM25 + HNSW) | ✅ OK | Used at session start (`rulebook_memory_search`) and end (`rulebook_session_end`) |
| Decisions / knowledge / learnings | ✅ OK | Captured under `.rulebook/{decisions,knowledge,learnings}/` |
| Cortex bootstrap promotes `.rulebook/*` | ✅ OK | Default-discovery (commit fc87b4d); laws, learnings, decisions automatically indexed as Cortex entities |

## Environment Variables (docker-compose.yml excerpt)

**Key Cortex overrides:**

```
CORTEX_INGESTION_BIND=0.0.0.0:17010
CORTEX_SYNAP_URL=http://synap:15500
CORTEX_ARCHIVE_ROOT=/var/lib/cortex/archive
CORTEX_EMBEDDER_VECTORIZER_URL=http://vectorizer:15002
CORTEX_EMBEDDER_VECTORIZER_USER=admin (default)
CORTEX_EMBEDDER_VECTORIZER_PASSWORD=cortex-dev-admin (default)
CORTEX_EMBEDDER_DIM=512
CORTEX_GRAPH_NEXUS_URL=http://nexus:15474
CORTEX_GRAPH_CYPHER_DIR=/opt/cortex/cypher
CORTEX_FULLTEXT_MEILI_URL=http://meilisearch:7700
CORTEX_FULLTEXT_MEILI_API_KEY=cortex-dev-master-key (default)
CORTEX_VECTORIZER_JWT_WARMUP_SECS=0
CORTEX_RULEBOOK_ROOTS=/workspaces/Cortex/.rulebook,... (multi-repo roots)
CORTEX_COVERAGE_SLUGS_ONLY=cortex,vectorizer,nexus,synap,rulebook,...
```

## Summary Health Heatmap (2026-04-28)

| Service | Health | Severity | Workaround | Owner |
|---------|--------|----------|-----------|-------|
| Vectorizer | 🟡 | Medium | Trust queryable signal; instrument post-verify | HiveLLM/Vectorizer |
| Nexus | 🟡 | Medium | Per-row Cypher + post-assert | HiveLLM/Nexus |
| Synap | 🟢 | N/A | N/A | HiveLLM/Synap |
| Meilisearch | 🟡 | Medium | Diagnose worker offset; cleanup legacy | HiveLLM/Cortex (phase4a) |
| Claude (Haiku/Sonnet) | 🟢 | N/A | N/A | Anthropic |
| Rulebook | 🟢 | N/A | N/A | HiveLLM/Rulebook |
