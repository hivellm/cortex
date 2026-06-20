# Manual E2E feature verification

Run each probe against the live docker stack. Check `[x]` only when the
actual result matches **Expect**. On mismatch: paste the real output
inline, mark `[!]`, and open a follow-up rulebook bug task. Rebuild +
recreate the relevant image first so the container matches HEAD.

Conventions: `API=http://localhost:17000`. Probes use `curl`; MCP tools
via the `mcp__cortex__*` surface.

## Run 1 — 2026-06-20 (partial)

Rebuilt + recreated cortex-api at HEAD. Findings:
- PASS: §0.2 (12 healthy), §4.2/§4.3 issue#4 Bug1 (tool-calls/events no-repo → 200 empty, was 502), §4.4 (repo=api → real hits), §6.1 (scoped query fused), §8.1 status, §8.4 decisions/search, §9.1 overview, §9.2 sessions, §9.3 graph, §9.4 coverage+trust, §11 laws, §8.7 violations, §12.6 lru audit (exit 0).
- FOUND+FIXED: §6.2 issue#4 Bug2 — Windows cwd `E:\HiveLLM\Rulebook` resolved to `e-hivellm-rulebook` (std::path::Path on the Linux daemon ignores backslash). Fixed (split on both separators), redeployed, re-verified → `rulebook`. Commit 5f64291.
- PASS (run 1b): all 17 dashboard/retention/health GETs → 200 (analyses, classifications, conversations, consolidations, decisions, handoffs, memory, producers, tasks, tasks/summary, tools/stats, timeline/recent, active-work, retention/state+sweeps, health/versions+config); §6 query intents decision_lookup + similar_problems → 200; §10.1 branch list → 200.
- FOUND (follow-up phase0_missing-index-empty-200-all-handlers): §8.5 consolidations/recent + consolidations/search + ~8 other Meili-backed handlers still 502 on missing index (issue#4 Bug1 fix reached only 2 of ~12 handlers).
- FOUND (follow-up phase0_decision-fulltext-title-body-mismapped): a SUBSET of `cortex_decisions` docs (the `01KQNYF4J*` ingest batch) have `title==id` + no `decision_title`, while newer docs (`01KQNYMYKH`) are correct. Refined via two MCP probe paths: the index is a mix of malformed + correct docs (stale batch from an earlier ingest), NOT the search handler (returns raw hits) and NOT all docs → reindex.
- NOTE (not a bug): `/v1/search/vector` with `query_text` returns 400 `not_implemented` (server-side embedding not wired; caller must pass `query_vector`). classifier synap-consume WARN at 14:01 was transient (recovered, RestartCount=0).

## Run 1c — MCP tool surface (§8) via mcp__cortex__*
- PASS: cortex_pre_thinking (bundle, 181ms, fail_open=false), cortex_active_work (lists all tasks incl. phase0/27/28), cortex_keyword_search (raw Meili, correct decision docs), cortex_decision_search, cortex_query (free_search/decision_lookup/similar_problems), cortex_status.
- EMPTY (note, re-probe with seeded data; not confirmed bugs): cortex_timeline (cortex:main — no TimelineEvent rows), cortex_topic_search (repo:cortex — no topic cards matched), cortex_similar_sessions (cortex — none ≥0.6 floor).
- PASS: cortex_graph_query neighbors (node 01KQVHJ5... → Turn neighbor + HAS_TOOL_CALL edge, live traversal); cortex_capture_memory WRITE (event_id returned, doc lands in cortex-cortex-misc within ~35s).
- FOUND+FIXED (phase0_captured-memory-not-retrievable-via-query): captured memory was indexed in `cortex-<repo>-misc` but `cortex_query` free_search didn't fan out to that family. Fixed free_search to fan out across code+docs+misc; redeployed; round-trip re-verified live (zeta-7731 now FOUND). Commit e3d87c6.
- PENDING: cortex_feedback_record→signals, cortex_audit{query_id}, cortex_history/supersession, cortex_session_timeline model-name (needs adapter redeploy + fresh session), §5.2 phase27a edge confidence (needs graph-worker rebuild).
- PENDING NEXT RUN: §8.2 model-name in timeline (needs adapter redeploy + a fresh session — old archived events have model=None); §5.2 phase27a edge confidence (needs graph-worker rebuild); §5.1/§8.11 re-probe with correct request shape.

## 0. Pre-flight: deploy HEAD + stack health
- [ ] 0.1 Rebuild + recreate images that lag HEAD (`docker compose build cortex-api cortex-graph-worker cortex-adapter*`; `up -d --force-recreate`) — Expect: all containers `Up (healthy)`
- [ ] 0.2 `docker ps` — Expect: classifier/api/nexus/graph/ingestion/synap/embedder/fulltext/vectorizer/meili all `Up (healthy)`, RestartCount stable
- [ ] 0.3 `curl $API/v1/health` — Expect: 200
- [ ] 0.4 `curl $API/v1/health/freshness` + `/v1/health/config` + `/v1/health/coverage` — Expect: 200, no `error` severity that's new

## 1. Ingestion + capture (spec 01/02/10/25)
- [ ] 1.1 Post a synthetic event `POST $API/v1/events` (or `/v1/events/batch`) — Expect: 2xx, event id returned
- [ ] 1.2 Confirm it lands in the archive (parquet) and is queryable via `cortex_status` event count delta — Expect: count increments
- [ ] 1.3 Adapter hook fires on a real Claude Code turn (UserPromptSubmit/Stop) — Expect: turn envelope captured (check `/v1/dashboard/sessions`)

## 2. Classifier worker (spec 05)
- [ ] 2.1 `docker logs cortex-classifier-worker --tail 5` — Expect: `mode: Static`, synap workers started, NO restart loop (regression guard for the idle fix)
- [ ] 2.2 Ingest a tool_call event → confirm classifier enriches it (kind/topics) — Expect: enriched event downstream

## 3. Embedder + Vectorizer lane (spec 06)
- [ ] 3.1 `cortex_vector_search` for a known symbol — Expect: ≥1 hit with score
- [ ] 3.2 Confirm per-repo collections exist in Vectorizer (`/api/collections` or status) — Expect: `cortex-<slug>-code` present

## 4. Fulltext + Meili lane (spec 08/22)
- [ ] 4.1 `cortex_keyword_search` on `cortex_decisions` — Expect: ≥1 hit, NO 91KB oversized line (issue#4 cap: fields capped when attributes_to_retrieve omitted)
- [ ] 4.2 `POST $API/v1/search/tool-calls {"q":"","limit":5}` with NO repo, on a daemon missing the global index — Expect: **200 with empty hits** (issue#4 Bug1 fix; was 502)
- [ ] 4.3 `POST $API/v1/search/events {"kind":"turn"}` no repo, missing index — Expect: **200 empty** (was 502)
- [ ] 4.4 Same with a real `repo` that IS indexed — Expect: 200 with hits

## 5. Graph lane + Nexus (spec 07)
- [ ] 5.1 `cortex_graph_query` (edge_artifact_touched_neighbours) for a known artifact — Expect: ≥1 neighbour
- [ ] 5.2 Query Nexus for a recent edge's props — Expect: **`confidence` + `confidence_score` present** (phase27a); `Extracted` on AST/structural edges, `Inferred` on classifier relations
- [ ] 5.3 `cortex_files_touched` / `cortex_history` for a file — Expect: results

## 6. Query API + fusion (spec 11/27)
- [ ] 6.1 `cortex_query {intent:"free_search", scope.repo:"cortex"}` — Expect: fused hits (snippets), `scope_resolved.repo` = `cortex`
- [ ] 6.2 `cortex_query` from nested cwd with NO scope (issue#4 Bug2) — Expect: slug resolves to basename (e.g. `rulebook`), NOT `e-hivellm-rulebook`; no false `repo_not_indexed`
- [ ] 6.3 `cortex_query_explain` — Expect: per-lane contribution breakdown (BM25/dense/graph + RRF)
- [ ] 6.4 Reranker active (if endpoint set): confirm `reranker.fallback` NOT emitted on healthy path; order changes vs no-rerank (spec 27)
- [ ] 6.5 Phantom-link verifier: a snippet citing a renamed/deleted symbol — Expect: `verified:false` + verdict (spec 28)

## 7. Pre-thinking (spec 12)
- [ ] 7.1 `cortex_pre_thinking {user_prompt, cwd}` — Expect: budgeted bundle (laws + snippets + decisions), within byte budget
- [ ] 7.2 Recent-files-in-scope: edit a file, re-run — Expect: file appears in scope

## 8. MCP tool surface (spec 20) — smoke each
- [ ] 8.1 `cortex_status` — Expect: daemon up, indexed_repos list
- [ ] 8.2 `cortex_session_timeline` for the live session — Expect: rows; **`deltas.model` shows the real model name** (claude-*), NOT absent/`claude-code` (model-name fix) — for events captured AFTER the adapter rebuild
- [ ] 8.3 `cortex_timeline` / `cortex_session_replay` — Expect: ordered events
- [ ] 8.4 `cortex_decision_search` + `cortex_decision_chain` — Expect: decisions + supersession chain
- [ ] 8.5 `cortex_consolidations` / `_recent` / `_search` / `_by_entity` / `_diff` / `_get` / `_lineage` / `_costs` — Expect: 2xx each
- [ ] 8.6 `cortex_topic_search` — Expect: topic cards
- [ ] 8.7 `cortex_law_violations` — Expect: 2xx (list or empty)
- [ ] 8.8 `cortex_similar_sessions` / `cortex_active_work` / `cortex_files_touched` — Expect: 2xx
- [ ] 8.9 `cortex_capture_memory` then `cortex_query` for it — Expect: memory retrievable
- [ ] 8.10 `cortex_feedback_record` then `cortex_feedback_signals` — Expect: signal persisted
- [ ] 8.11 `cortex_audit {query_id}` from a prior query — Expect: audit envelope
- [ ] 8.12 `cortex_missing` / `cortex_unknown` — Expect: 2xx

## 9. Dashboard endpoints (spec 16/21/28)
- [ ] 9.1 `/v1/dashboard/overview` — Expect: events_total, repos_indexed, kind_breakdown
- [ ] 9.2 `/v1/dashboard/sessions` + `/conversations/{id}` + `/summary` — Expect: live session
- [ ] 9.3 `/v1/dashboard/graph` — Expect: nodes+edges (incl. confidence once 27a deployed)
- [ ] 9.4 `/v1/dashboard/coverage` + `/canary` + `/producers` + `/trust` — Expect: 2xx
- [ ] 9.5 `/v1/dashboard/timeline/recent` + SSE `/stream` — Expect: events stream
- [ ] 9.6 GUI loads against the API; api.generated.ts contract matches (spec 28) — Expect: no console contract errors

## 10. Temporal / branches / cross-project (spec 30-35)
- [ ] 10.1 `/v1/branch/{project}` list + show — Expect: 2xx
- [ ] 10.2 `cortex_query` with `as_of` / `include_history` — Expect: bitemporal filter applied
- [ ] 10.3 `cortex_supersession` / `/v1/entity/{id}/history` — Expect: lifecycle states

## 11. Retention / governance (spec 13/14/19)
- [ ] 11.1 `cortex-ops` retention sweep dry-run — Expect: plan, no crash
- [ ] 11.2 Laws DSL: a denied action (e.g. `--no-verify`) evaluates to deny — Expect: violation recorded
- [ ] 11.3 `/v1/admin/forget` dry-run path — Expect: gated by confirmation token

## 12. Recent-fix regression guards (consolidated)
- [ ] 12.1 issue#4 Bug1 (404→empty) — covered by 4.2/4.3
- [ ] 12.2 issue#4 Bug2 (cwd→slug basename) — covered by 6.2
- [ ] 12.3 model-name in timeline — covered by 8.2
- [ ] 12.4 classifier idle (no restart loop) — covered by 2.1
- [ ] 12.5 phase27a edge confidence — covered by 5.2
- [ ] 12.6 lru RUSTSEC-2026-0002 — `cargo audit --deny warnings` exits 0

## 99. Tail (mandatory)
- [ ] 99.1 Documentation: record the run outcome (pass/fail per area) in this task + CHANGELOG note if defects found
- [ ] 99.2 Tests: every defect found here becomes a regression test in the owning crate
- [ ] 99.3 Run: re-run the failed probes after each fix until the whole checklist is green
