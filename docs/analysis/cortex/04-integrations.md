# 04 — Integrations (Vectorizer, Nexus, Synap, Meili, Claude)

Cortex is an orchestrator: it composes existing services. Health here means "does the integration boundary work end-to-end, and where do drifts surface?"

## Vectorizer (HivehubCloud, image `hivehub/vectorizer:3.0.0`)

| Aspect                      | State                                                                                          |
|-----------------------------|------------------------------------------------------------------------------------------------|
| Auth (`/auth/login` → JWT)  | ✅ SDK 3.0.3 path works once the env var actually contains a JWT.                              |
| `create_collection` / info  | ✅ SDK call works.                                                                             |
| `insert_texts`              | ⚠️ Server reports `total_failed=4-5/64`, `vector_count=0`. Vectors *are* queryable downstream — server response is misleading or partial-success path is undocumented. |
| `search_vectors`            | ✅ Used by `VectorLane`; returns ranked hits with scores. Audit "classifier worker" probe top-1 = 0.136 (BM25-as-embedding-style score, weak). |
| `list_vectors` / `get`      | ❌ Server returns synthetic 200 for any id; SDK has no list path. Worked around by `LiveVectorizerClient::list_stored_chunk_ids`. |

**Recorded ADR:** [001 — bypass vectorizer-sdk for /insert and /get_vector](../../../.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md). Status: **superseded** for `login` and `insert` (SDK 3.0.3 closed those drifts) but the `get_vector` path is still bypassed.

**Recorded knowledge:** the ongoing SDK-server drift is tracked under [vectorizer-sdk-3-0-3-follow-up](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-follow-up-2-of-6-drifts-resolved-3-4-5-6-still-open-server-side.md) and [round-8-follow-up](../../../.rulebook/knowledge/anti-patterns/vectorizer-sdk-3-0-3-round-8-follow-up-server-assigned-uuids-accepted-drift-4-neutralised.md). 2 of 6 drifts resolved; 3, 4, 5, 6 still open server-side. The user has been clear that the SDK is **not** to blame for our integration bugs (memory: [feedback_dont_blame_hive_services](../../../../C:/Users/Bolado/.claude/projects/e--HiveLLM-Cortex/memory/feedback_dont_blame_hive_services.md)) — but the `total_failed` reporting is an upstream contract we have to either trust or instrument around.

**Action:** decide whether to (a) trust the queryable-downstream signal and ignore `total_failed`, (b) add a post-upsert verification that fetches and confirms a vector landed, or (c) wait for SDK 3.0.4 / server fix. Option (b) is what `cortex doctor consistency` would do.

## Nexus (image `hivehub/nexus:1.15.0`)

| Aspect                      | State                                                                                          |
|-----------------------------|------------------------------------------------------------------------------------------------|
| Health endpoint             | ✅ `/health`.                                                                                   |
| Cypher single-statement writes | ✅ Per-row Cypher renderers (commit `1e34417`) work.                                          |
| `UNWIND` batch writes       | ❌ **Silently drop** — recorded in [Cypher UNWIND-write knowledge anti-pattern](../../../.rulebook/knowledge/anti-patterns/cypher-unwind-write-and-param-write-substitution-silently-drop-in-nexus-1-15.md). |
| Param-write substitution    | ❌ Same drift as UNWIND; substitution silently no-ops on writes.                                |
| Driver reports success      | ⚠️ `nodes_upserted=271` reported but `MATCH (n)` returned 1 unlabeled node post-bootstrap.    |
| Detection                   | ✅ `assert_write_landed` (commit `5bd0185`) now surfaces the drop instead of swallowing it.    |

**The fix actually applied:** Cortex stopped sending UNWIND-style batches and now emits per-row Cypher with explicit parameter binding. After commit `5bd0185`, post-write read-back asserts the row exists. This is reactive rather than upstream-resolved — if Nexus 1.15 fixes UNWIND, the per-row path becomes wasteful but safe.

**Action:** keep the per-row path; track Nexus releases for UNWIND fix; add an integration test that asserts an `(n:Symbol)-[:DEFINES]->(:Artifact)` write lands once phase4c emits those edges.

## Synap (image `hivehub/synap:latest`)

| Aspect                  | State                                                                                          |
|-------------------------|------------------------------------------------------------------------------------------------|
| Stream consume          | ✅                                                                                             |
| Stream publish          | ✅                                                                                             |
| Room not found at boot  | ✅ Auto-create on first 404 ([pattern](../../../.rulebook/knowledge/patterns/synap-publisher-should-auto-create-rooms-on-first-not-found.md)). |
| Pub/sub for SSE         | ✅ Live timeline stream uses Synap → cortex-api SSE bridge (commit `ac10b5e`).                 |

This is the most boring/reliable integration. Synap behaves; the only past gotcha was a chicken-and-egg with stream creation, fixed by the auto-create-on-404 pattern.

## Meilisearch (image `getmeili/meilisearch:v1.10`)

| Aspect                            | State                                                                                          |
|-----------------------------------|------------------------------------------------------------------------------------------------|
| Index create / settings PATCH     | ✅ Once tooling-only fields are stripped at the client boundary. See [knowledge anti-pattern](../../../.rulebook/knowledge/anti-patterns/don-t-bake-tooling-only-fields-into-json-payloads-sent-verbatim-to-a-strict-downstream.md). |
| Documents upsert with `primaryKey=id` | ✅ Pinned in commit `99377a1`.                                                              |
| Search                            | ✅ `MeiliKeywordLane` returns results with score + source attribution.                          |
| Index naming                      | ✅ `cortex-{repo}-{family}` enforced by [routing.rs:105-107](../../../crates/cortex-fulltext/src/routing.rs#L105-L107). |
| Stale legacy indexes              | ❌ 6 empty indexes from pre-slug naming, not cleaned up. Phase4a sweep is the planned fix.     |
| Cross-repo coverage               | ❌ Only `Cortex` repo populated despite three repos walked. Worker offset / consumer state suspected. |

## Claude (Haiku via CLI, Sonnet via CLI/API)

| Aspect                              | State                                                                                          |
|-------------------------------------|------------------------------------------------------------------------------------------------|
| Haiku CLI for per-event classify    | ✅ `claude -p "$PROMPT" --model claude-haiku-4-5 --output-format json` — but **mostly disabled** in favor of `StaticClassifier`. |
| `--max-tokens` with CLI 2.x         | ✅ Dropped (commit `c41dab0`) — flag was broken in newer CLI.                                  |
| Sonnet for cross-event analysis     | ✅ Commit `a62fcbd`. Two-mode: CLI when on PATH, direct Anthropic API otherwise (CI/server).   |
| Fallback                            | ✅ Analyzer config falls back gracefully if neither CLI nor API key is available.              |

Note from [analyzer.rs:46-58](../../../crates/cortex-api/src/analyzer.rs): the API-key path was added because the user's setup hides the `claude` binary inside Cursor/VS Code — so the Cortex daemon (running outside those environments) cannot exec it. Direct API call sidesteps that entirely.

## Rulebook (sister project, used as a library + MCP server)

| Aspect                              | State                                                                                          |
|-------------------------------------|------------------------------------------------------------------------------------------------|
| Task management (`mcp__rulebook__*`)| ✅ Mandatory for task lifecycle per [CLAUDE.md](../../../CLAUDE.md).                            |
| Memory (BM25 + HNSW hybrid)         | ✅ Used at session start (`rulebook_memory_search`) and end (`rulebook_session_end`).           |
| Decisions / knowledge / learnings   | ✅ Captured under `.rulebook/{decisions,knowledge,learnings}/`.                                |
| Cortex bootstrap promotes `.rulebook/*` | ✅ Default-discovery (commit `fc87b4d`).                                                    |

## Summary heatmap

| Service     | Bus        | Health    | Drift severity | Workaround in place | Action                            |
|-------------|-----------|-----------|----------------|---------------------|-----------------------------------|
| Vectorizer  | direct    | OK        | Medium         | yes (per-row + verify) | post-upsert verification, doctor |
| Nexus       | direct    | OK        | Was high, now contained | yes (assert_write_landed) | track 1.15 → 1.16 release for UNWIND |
| Synap       | direct    | OK        | none           | n/a                 | none                              |
| Meilisearch | direct    | OK        | n/a (settings) | yes (strip tooling) | drop stale indexes; replay missing repos |
| Claude (Hk) | CLI       | Mostly OK | low            | static fallback     | none — Haiku is intentionally limited |
| Claude (Sn) | CLI + API | OK        | none           | dual-mode           | rate-limit + cost telemetry       |
| Rulebook    | MCP       | OK        | none           | n/a                 | none                              |
