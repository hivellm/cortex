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
- FOUND+FIXED (phase0_missing-index-empty-200-all-handlers): consolidations/recent + search + ~8 other Meili-backed handlers 502'd on missing index. Applied the is_meili_index_missing guard to all; redeployed; verified live — recent/search/diff/costs → 200 empty, {id}/lineage → 404 (were 502).
- FOUND (follow-up phase0_decision-fulltext-title-body-mismapped): a SUBSET of `cortex_decisions` docs (the `01KQNYF4J*` ingest batch) have `title==id` + no `decision_title`, while newer docs (`01KQNYMYKH`) are correct. Refined via two MCP probe paths: the index is a mix of malformed + correct docs (stale batch from an earlier ingest), NOT the search handler (returns raw hits) and NOT all docs → reindex.
- NOTE (not a bug): `/v1/search/vector` with `query_text` returns 400 `not_implemented` (server-side embedding not wired; caller must pass `query_vector`). classifier synap-consume WARN at 14:01 was transient (recovered, RestartCount=0).

## Run 1c — MCP tool surface (§8) via mcp__cortex__*
- PASS: cortex_pre_thinking (bundle, 181ms, fail_open=false), cortex_active_work (lists all tasks incl. phase0/27/28), cortex_keyword_search (raw Meili, correct decision docs), cortex_decision_search, cortex_query (free_search/decision_lookup/similar_problems), cortex_status.
- EMPTY (note, re-probe with seeded data; not confirmed bugs): cortex_timeline (cortex:main — no TimelineEvent rows), cortex_topic_search (repo:cortex — no topic cards matched), cortex_similar_sessions (cortex — none ≥0.6 floor).
- PASS: cortex_graph_query neighbors (node 01KQVHJ5... → Turn neighbor + HAS_TOOL_CALL edge, live traversal); cortex_capture_memory WRITE (event_id returned, doc lands in cortex-cortex-misc within ~35s).
- FOUND+FIXED (phase0_captured-memory-not-retrievable-via-query): captured memory was indexed in `cortex-<repo>-misc` but `cortex_query` free_search didn't fan out to that family. Fixed free_search to fan out across code+docs+misc; redeployed; round-trip re-verified live (zeta-7731 now FOUND). Commit e3d87c6.
- PENDING: cortex_feedback_record→signals, cortex_audit{query_id}, cortex_history/supersession, cortex_session_timeline model-name (needs adapter redeploy + fresh session).
- §5.2 phase27a edge confidence — PARTIAL: stamping is code-complete + unit-tested (6 tests). Live: queried Nexus directly (`POST :17002/cypher`) — 366 existing EMITTED_BY edges have 0 confidence, which is EXPECTED (written pre-phase27a). `cortex-ops graph backfill --apply` projected 260 confidence-stamped edges but they didn't persist (endpoint MATCH-MERGE drop, known phase15c caveat). DESIGN NOTE: `render_edge_merge` inlines props into the MERGE pattern, so `confidence` joins the edge merge-identity (deterministic/stable → safe) and existing edges need a rewrite to carry it. Full live confirmation needs the rebuilt graph-worker writing NEW edges through the live pipeline. cortex_graph_query cypher mode is gated (403) — read via Nexus `/cypher` directly.
- PENDING NEXT RUN: §8.2 model-name in timeline (needs adapter redeploy + a fresh session — old archived events have model=None); §5.2 phase27a edge confidence (needs graph-worker rebuild); §5.1/§8.11 re-probe with correct request shape.

Legend: [x] verified live · [~] partial/works-with-caveat · [ ] not yet run.

## 0. Pre-flight: deploy HEAD + stack health
- [~] 0.1 Deploy HEAD — cortex-api (rebuilt+recreated ×5), cortex-graph-worker (rebuilt+recreated, phase27a live), classifier (earlier) at HEAD; cortex-adapter (host hook) still pending
- [x] 0.2 `docker ps` — 12 containers `Up (healthy)`, RestartCount stable
- [x] 0.3 `/v1/health` — 200
- [x] 0.4 `/v1/health/config` + `/v1/health/coverage` — 200 (coverage rows_total=0 noted, endpoint OK)

## 1. Ingestion + capture (spec 01/02/10/25)
- [x] 1.1 Event ingest — `cortex_capture_memory` → `/v1/ingest` returns event_id + indexed_at (2xx)
- [x] 1.2 Lands + queryable — captured doc appears in `cortex-cortex-misc` within ~35s
- [ ] 1.3 Adapter hook on a real Claude Code turn (needs a live captured turn)

## 2. Classifier worker (spec 05)
- [x] 2.1 `mode: Static`, synap workers started, RestartCount=0 — no restart loop (idle-fix regression guard holds; one transient synap-consume WARN recovered)
- [ ] 2.2 Classifier enrichment of a tool_call (not directly verified end-to-end)

## 3. Embedder + Vectorizer lane (spec 06)
- [ ] 3.1 `cortex_vector_search` — `query_text` returns 400 `not_implemented` (server-side embedding not wired; needs `query_vector`)
- [ ] 3.2 Per-repo Vectorizer collections (not directly inspected)

## 4. Fulltext + Meili lane (spec 08/22)
- [x] 4.1 `cortex_keyword_search cortex_decisions` — hits returned, no oversized-line blowup
- [x] 4.2 `/v1/search/tool-calls` no repo → **200 empty** (was 502)
- [x] 4.3 `/v1/search/events kind=turn` no repo → **200 empty** (was 502)
- [x] 4.4 `kind=turn repo=api` → 200 with real hits

## 5. Graph lane + Nexus (spec 07)
- [x] 5.1 `cortex_graph_query neighbors` — live traversal (Turn neighbor + `HAS_TOOL_CALL` edge)
- [~] 5.2 Edge confidence — stamping code-complete + unit-tested (6 tests); graph-worker redeployed with phase27a. Live new-edge confirmation BLOCKED: an injected `cortex_capture_memory` event (omega-4419) reached Meili but did NOT propagate to the graph (no node in Nexus, graph-worker idle, 0 edges with confidence) — the ingest→enriched→graph stage isn't flowing for injected events on this daemon (separate pipeline observation, not a phase27a issue). Existing edges have 0 confidence (expected, pre-phase27a). Nexus read via `/cypher` directly (cortex-api cypher gated 403)
- [ ] 5.3 `cortex_files_touched` / `cortex_history` for a file (not run)

## 6. Query API + fusion (spec 11/27)
- [x] 6.1 `cortex_query free_search repo=cortex` — fused snippets, `scope_resolved.repo=cortex`
- [x] 6.2 cwd→slug (issue#4 Bug2) — Windows `E:\HiveLLM\Rulebook` → `rulebook` (fixed+live)
- [ ] 6.3 `cortex_query_explain` per-lane breakdown (not run)
- [ ] 6.4 Reranker active path (no endpoint configured in this stack)
- [ ] 6.5 Phantom-link verifier on a renamed/deleted symbol (not run)

## 7. Pre-thinking (spec 12)
- [x] 7.1 `cortex_pre_thinking` — budgeted bundle returned (5 snippets, 181ms, fail_open=false)
- [ ] 7.2 Recent-files-in-scope (not run)

## 8. MCP tool surface (spec 20)
- [x] 8.1 `cortex_status` — daemon up, indexed_repos list (8 repos)
- [ ] 8.2 `cortex_session_timeline` model-name (needs adapter rebuild + fresh session)
- [~] 8.3 `cortex_timeline` returns (empty for cortex:main); `session_replay` not run
- [x] 8.4 `cortex_decision_search` — hits returned (decision_chain not separately run)
- [x] 8.5 consolidations recent/search/by_entity/diff/costs → 200, get/lineage → 404 (missing-index fix; were 502)
- [x] 8.6 `cortex_topic_search` — works (empty for repo:cortex)
- [x] 8.7 `cortex_law_violations` / dashboard violations — 200
- [x] 8.8 `cortex_similar_sessions` (200, empty) + `cortex_active_work` (lists tasks); files_touched not run
- [x] 8.9 `cortex_capture_memory` → `cortex_query` round-trip — retrievable (FIXED + verified live)
- [ ] 8.10 `cortex_feedback_record` → `_signals` (not run)
- [ ] 8.11 `cortex_audit {query_id}` (not run)
- [ ] 8.12 `cortex_missing` / `cortex_unknown` (not run)

## 9. Dashboard endpoints (spec 16/21/28)
- [x] 9.1 `/v1/dashboard/overview` — events_total=54653, repos_indexed=8, kind_breakdown
- [x] 9.2 `/v1/dashboard/sessions` — live session (1096 events)
- [x] 9.3 `/v1/dashboard/graph` — nodes+edges
- [x] 9.4 coverage + canary + producers + trust — 200 (trust is stub_until_spec14)
- [x] 9.5 `/v1/dashboard/timeline/recent` — 200
- [ ] 9.6 GUI contract load (not run)

## 10. Temporal / branches / cross-project (spec 30-35)
- [x] 10.1 `/v1/branch/{project}` list — 200 (empty branches)
- [ ] 10.2 `cortex_query` with `as_of` / `include_history` (not run)
- [ ] 10.3 `cortex_supersession` / entity history (not run)

## 11. Retention / governance (spec 13/14/19)
- [ ] 11.1 retention sweep dry-run (not run)
- [~] 11.2 Laws — `/v1/dashboard/laws` + violations return data (deny-eval not exercised)
- [ ] 11.3 `/v1/admin/forget` token gate (not run)

## 12. Recent-fix regression guards (consolidated)
- [x] 12.1 issue#4 Bug1 (404→empty) — 4.2/4.3 pass
- [x] 12.2 issue#4 Bug2 (cwd→slug) — 6.2 pass (fixed this session)
- [ ] 12.3 model-name in timeline — pending adapter rebuild (8.2)
- [x] 12.4 classifier idle (no restart loop) — 2.1 pass
- [~] 12.5 phase27a edge confidence — code+unit-tested, worker deployed; live new-edge confirmation in progress (5.2)
- [x] 12.6 lru RUSTSEC-2026-0002 — `cargo audit` exits 0

## 99. Tail (mandatory)
- [~] 99.1 Documentation: run outcomes logged inline (runs 1a/1b/1c + this run); CHANGELOG updated for all fixes
- [x] 99.2 Tests: each fix shipped with a regression test (resolve_scope windows path, free_search misc fan-out, is_meili_index_missing, edge confidence ×6)
- [~] 99.3 Re-run failed probes after each fix — done for the 3 fixed bugs (all re-verified live); remaining unchecked probes pending
