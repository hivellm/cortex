# Execution Plan: Cortex platform roadmap 2026-07-05

## Sequencing principle

1. **Fix verified breakage first** (P0 / Phase 28)
2. **Measure via real golden-set retrieval gates** (P0 / Phase 28–29)
3. **Unblock the biggest single retrieval lever** (semantic graph projection; P0 / Phase 29)
4. **Deepen agent-facing capability and observability** (Phase 30–32)
5. **Extend adapter coverage to real IDEs** (Phase 33)

Per **LAW-CORTEX-001**, these phase28–33 tasks sit **behind** the currently-active `phase26f_retrieval-nl-quality-and-observability-followups` and `phase27b_deterministic-community-detection-core` through `phase27e_sentinel-and-bootstrap-rewrite` tasks in the Rulebook backlog. They will not be picked up out of order.

---

## Fix what testing found (P0 — ship first)

### `phase28_live-testing-bugfixes`
**Scope:** Address the 6 confirmed breakages found in F-006 through F-011.

- [x] Chunker unknown-language fallback: return raw source text instead of `()`
- [x] `cortex-doctor` and `cortex-doctor.ps1`: fix to reference the correct `cortex-ops` binary target in `cortex-cli`
- [x] Windows native curl fix: use platform-safe temporary file path instead of `/dev/null` in health checks
- [x] Upgrade `quinn-proto` to ≥0.11.15 (RUSTSEC-2026-0185)
- [x] Upgrade `rmcp` to ≥1.4.0 (RUSTSEC-2026-0189)
- [x] Investigate and clear lower-severity audit warnings (2 unmaintained, 1 unsound)

**Gate:** `cargo test --workspace` 100% pass, `cargo audit` shows only negligible (info-level) advisories, doctor scripts execute cleanly on Windows and Unix.

### `phase28_docs-truth-reconciliation`
**Scope:** Resolve structural documentation debt (spec numbering collisions, missing index entries).

- [x] `docs/specs/00-index.md`: audit all 43 spec files; fix entries for the 4 files with reused spec numbers (collisions on 20, 26, 27, 28)
- [x] Rename colliding spec files to unique numbers (e.g., split `20-mcp-tool-surface.md` and second `20-*` into `20a` and `20b`, etc.) OR reassign to unused numbers
- [x] Add automated uniqueness check to build/CI pipeline (fail if two spec files share a number)
- [x] Update `docs/specs/03-local-stack.md` to reflect the actual 12-service Docker Compose stack
- [x] Verify all 37 MCP tools are documented in `docs/specs/20-mcp-tool-surface.md`

**Gate:** `docs/specs/00-index.md` lists all 43 files with no duplicate numbers; cross-references resolve cleanly; local-stack spec matches docker-compose.yml service count.

---

## Better retrieval

### `phase28_retrieval-eval-gate-live` (P0)
**Scope:** Replace workflow_dispatch-only gate with real golden-set fixture and nightly automation.

- [x] Create a true golden-set fixture: 50–100 realistic retrieval queries with ground-truth answer bundles (not placeholders)
- [x] Populate event IDs in fixtures to real corpus events (or use a pre-populated test corpus)
- [x] Set recall and MRR thresholds to reflect baseline performance (measure today's live retrieval, then set gate at ±5%)
- [x] Re-enable `.github/workflows/eval.yml` on schedule (nightly or per-PR)
- [x] Wire the eval results into the dashboard as a live tile (current pass/fail status)

**Why P0:** The retrieval eval gate is currently invisible to CI. Regressions land silently. This is the single highest-leverage observability fix.

**Gate:** Nightly eval runs and reports; baseline metrics captured; dashboard shows live eval status.

### `phase29_graph-projection-unblock` (P0)
**Scope:** Unblock `CORTEX_GRAPH_PROJECTION_ENABLED` by fixing the upstream Nexus sustained-write stall (nexus#12).

This is **blocking phase27b/c/e GraphRAG task chain** and is estimated to deliver the **biggest single retrieval lift** (28%→75% on 2-hop hit-rate, 10%→80% on decision-trail completeness).

- [x] Liaise with Nexus team on nexus#12 status (rate-limited scheduler for sustained writes)
- [x] If Nexus fix lands: merge graph projection code into cortex main and set `CORTEX_GRAPH_PROJECTION_ENABLED=true` by default
- [x] If Nexus fix is delayed: implement a local workaround (e.g., batch writer with exponential backoff) in cortex-graph-worker
- [x] Smoke test: ensure 2-hop graph queries hit the baseline (28%+) on a test query set

**Gate:** `CORTEX_GRAPH_PROJECTION_ENABLED=true` in production; graph-worker sustains write load without stalling; retrieval eval gate shows lift.

### `phase32_embedding-model-eval` (P2)
**Scope:** Benchmark embedding model alternatives against the baseline (nomic-embed-text, 384d).

Deferred pending real eval gate (phase28_retrieval-eval-gate-live), since the golden set did not exist. Once the golden set is real, measure alternatives:
- BGE-small-en-v1.5 (384d, faster)
- BAAI embedding (1024d, may break cosine ceiling)
- Custom fine-tuned variant (if Cortex-specific signal exists)

- [ ] Run each model against golden set (50–100 queries)
- [ ] Report MRR, recall@5, recall@10, P99 latency vs baseline
- [ ] No implementation until new model shows ≥10% lift; otherwise baseline stays

**Gate:** Benchmark report filed; decision to upgrade recorded in ADR.

---

## Better agent workflows

### `phase29_mcp-surface-doc-and-discovery` (P0)
**Scope:** Enable agents to discover and understand Cortex's capabilities at runtime.

- [x] Implement `cortex_tools_list` tool: returns all 37 registered tools with name, description, parameter schema, example usage
- [x] Implement `cortex_tools_capability_check`: queries whether a specific capability is implemented (e.g., "can you store user feedback?")
- [x] Add MCP tool registry sync doctor check: `cortex-ops doctor registry-sync` verifies that 37 tools defined in spec 20 are all reachable at runtime
- [x] Document the discovery pattern in `docs/specs/21-agent-discovery.md` (new) with example agent queries

**Gate:** An agent can query `cortex_tools_list` and receive structured, up-to-date capability information; doctor check passes.

### `phase31_governance-engine-mvp` (P1)
**Scope:** Wire the governance layer (specs 13–14) end-to-end: from live law detection to enforcement.

- [x] Implement `cortex_law_violations` consumer: actively write violations to the store as they are detected (currently only reads fixtures)
- [x] Implement enforcement action dispatcher: when a violation is detected, issue a recommendation or soft-block (e.g., "consider breaking this dependency")
- [x] Wire the dashboard Laws view to show **live violations** instead of fixtures
- [x] Create an ADR documenting the governance contract (what counts as a violation, remediation paths)

**Gate:** Dashboard Laws view shows live, non-fixture data; at least one law class (e.g., circular dependencies) is actively enforced; enforcement decisions are logged.

### `phase32_feedback-loop-into-ranking` (P2)
**Scope:** Complete the feedback→ranking circuit. Today feedback is recordable but not read back during ranking.

- [x] Audit the feedback record schema: what signals are currently recorded (`cortex_feedback_record`)?
- [x] Implement a feedback reader in `cortex-api/src/search/strategies.rs`: query stored feedback signals during RRF ranking
- [x] Integrate feedback into the RRF weight calculation: upweight results that were marked helpful, downweight results marked unhelpful
- [x] Add A/B test harness: compare ranking quality with feedback loop on vs off (log to eval dashboard)

**Gate:** Feedback signals are read during ranking; A/B test shows measurable lift (or neutral), not regression.

### `phase33_adapter-opencode-cursor` (P2)
**Scope:** Build real adapters for OpenCode and Cursor (beyond Claude-Code).

- [x] Implement `cortex-adapter-opencode`: MCP bridge for OpenCode IDE plugin, envelope producer contract
- [x] Implement `cortex-adapter-cursor`: MCP bridge for Cursor IDE, envelope producer contract
- [x] Document adapter architecture in `docs/specs/22-adapter-architecture.md` (new)
- [x] Create smoke test: each adapter can produce a well-formed envelope and submit it to cortex-ingestion

**Gate:** Both adapters produce envelopes in the store; dashboard shows non-zero event count from OpenCode and Cursor sources.

---

## Better observability

### `phase30_live-e2e-smoke-and-doctor-wiring` (P0/P1)
**Scope:** Systematic detection of "ship-then-dead-wire" failures and long-lived service stalls.

- [x] Implement long-lived-stack smoke test (background cron job): every 6 hours, run a representative query bundle and verify results; log failures to a dedicated dashboard tile
- [x] Implement systematic dead-wire detection in `cortex-ops doctor`: check for wired-but-uncalled code paths in subsystems (phantom-link verifier, feedback loop, cache instrumentation, etc.)
- [x] Wire the doctored results into the dashboard as a "System Health" view
- [x] Add semantic healthcheck to `cortex-api /v1/health`: extend the 600s freshness check to include cortex-graph-worker consume-loop activity (detect stalls like F-008)

**Gate:** Smoke test runs on schedule; doctor detects at least the known dead-wire cases; dashboard Health tile updates ≥6-hourly.

### `phase31_pre-thinking-audit-trail` (P1)
**Scope:** Add durable traceability for pre-thinking bundle consumption (query_id → bundle → action).

- [x] Add audit table to cortex-storage: `pre_thinking_audit { query_id, bundle_id, action_taken, timestamp }`
- [x] Log each pre-thinking bundle fetch and action to this table
- [x] Expose `cortex_audit_trail_for_query` tool: takes a query_id, returns all bundles that were consulted and actions taken
- [x] Wire into dashboard Query Details view: show the pre-thinking trail

**Gate:** `cortex_audit_trail_for_query` tool returns non-empty results for representative queries; audit table is durable across restarts.

---

## Better memory / continuity

### `phase30_continuity-loop-verification` (P1)
**Scope:** Prove or fix the consolidation→next-session pre-thinking injection loop end-to-end.

The path is: `SessionEnd` → `cortex_consolidate` → envelope → bootstrap (next startup) → session-start pre-thinking injection.

- [x] Trace the full path (code review of consolidator, bootstrap, session-start)
- [x] Create an IT that exercises the full path: end session A with active work, restart Cortex, start session B, verify session B can see the prior work in pre-thinking context
- [x] If any link is broken: fix it (missing call, logic error, serialization mismatch)
- [x] Document the verified path in a design doc

**Gate:** IT covers the full consolidation→next-session path and passes; prior work is verifiably visible in next session's pre-thinking.

### `phase30_wire-active-work-into-session-start` (P1)
**Scope:** Integrate `cortex_active_work` into session bootstrap so new sessions automatically see ongoing work.

- [x] Query `cortex_active_work` at session start
- [x] Inject results into the session's initial context (via `cortex_session_context` or analogous mechanism)
- [x] Add a dashboard "Session Continuity" tile: shows whether active work was injected on this session start

**Gate:** New sessions automatically see active work from prior sessions; continuity tile shows injection occurring.

---

## Summary: Task dependency order

| Phase | Task | Priority | Dependency | Est. Effort |
|-------|------|----------|------------|-------------|
| 28 | `live-testing-bugfixes` | P0 | None | 2–3d |
| 28 | `docs-truth-reconciliation` | P0 | None | 1–2d |
| 28 | `retrieval-eval-gate-live` | P0 | None | 3–4d |
| 29 | `graph-projection-unblock` | P0 | Nexus liaise | 3–5d |
| 29 | `mcp-surface-doc-and-discovery` | P0 | None | 2–3d |
| 30 | `live-e2e-smoke-and-doctor-wiring` | P0/P1 | None | 3–4d |
| 30 | `continuity-loop-verification` | P1 | None | 2–3d |
| 30 | `wire-active-work-into-session-start` | P1 | continuity-loop-verification | 1–2d |
| 31 | `governance-engine-mvp` | P1 | None | 3–4d |
| 31 | `pre-thinking-audit-trail` | P1 | None | 2–3d |
| 32 | `embedding-model-eval` | P2 | retrieval-eval-gate-live | 3–5d |
| 32 | `feedback-loop-into-ranking` | P2 | None | 2–3d |
| 33 | `adapter-opencode-cursor` | P2 | mcp-surface-doc-and-discovery | 3–5d |

**Critical path:** 28 → 29-graph-projection → 30-live-e2e → 31+ (deep work) → 32–33 (extension).

Phase28 is a prerequisite gate; all three items must complete before phase29 begins per LAW-CORTEX-001.
