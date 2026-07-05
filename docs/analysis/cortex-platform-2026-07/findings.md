# Findings — Cortex platform audit (2026-07-05)

Method: a static code/spec/doc audit (exhaustive reads of `docs/specs/`, `crates/cortex-mcp-server/src/tools.rs`, `rulebook_decision_list`, `rulebook_knowledge_list`) combined with live verification performed the same day — `cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo audit`, and a live Docker-stack smoke test (health endpoints, doctor scripts, container logs). Every finding below traces to a command run or file read today; nothing is asserted from memory of an earlier analysis without being re-verified.

## Foundation / verified-working

**F-001 — Workspace type-checks clean.** `cargo check --workspace --all-targets` exits 0. One pre-existing unused-import warning in `cortex-cli` (not a blocker). *Verified: direct command execution.*

**F-002 — The Docker stack is actually running right now.** `docker compose ps` shows 12 services (cortex-api, cortex-ingestion, cortex-classifier-worker, cortex-embedder-worker, cortex-fulltext-worker, cortex-graph-worker, cortex-claude-archive, cortex-vectorizer, cortex-nexus, cortex-synap, cortex-meilisearch, cortex-reranker) plus a host-side `cortex-adapter-claude` daemon, most up continuously 8–13 days. This directly contradicted the old README's "no runnable binary in master" claim — that claim has been corrected in this same pass. *Verified: `docker compose ps`, container uptime, direct HTTP calls to cortex-api.*

**F-003 — 37 MCP tools registered and now fully documented.** `ToolRegistry::default_set()` in `crates/cortex-mcp-server/src/tools.rs` registers 37 tools; `docs/specs/20-mcp-tool-surface.md` previously documented only 7 and has been corrected in this pass. *Verified: direct source read, cross-checked against the server's own test asserting 37 tools.*

**F-004 — 43 spec files exist in docs/specs/.** `docs/specs/00-index.md` previously listed only 20 rows (specs 01-18, 20, 26) and has been corrected in this pass to document all 43. Most core specs (01-18) are 🟢 implemented; four spec numbers (20, 26, 27, 28) are each reused by two different files — a real numbering collision, tracked as its own task rather than silently renamed. *Verified: exhaustive Glob + read of every file in docs/specs/.*

**F-005 — 36 ADRs recorded.** *Verified: `rulebook_decision_list`.*

## Bugs found via live testing (new today, not previously known)

**F-006 — One real test failure: chunker fallback returns unit instead of raw text.** `cargo test --workspace` fails (exit 101, fail-fast aborted after the first failure). The single failure is `cortex-workers::embedder_it_chunk_pipeline::unknown_language_falls_back` (`crates/cortex-workers/tests/embedder_it_chunk_pipeline.rs:182`) — root-caused during task authoring to `nl_projection()`'s `ToolCall` branch in the chunker fallback path, which only reads `tool_name`/`input`/`output` and produces the literal string `"()"` for this payload shape instead of falling through to raw source text. At least 1,671 tests passed before the fail-fast abort; the true full-workspace count is unknown since the run stopped early. *Verified: direct `cargo test` output + source-level root-cause trace.*

**F-007 — Two high-severity RUSTSEC advisories.** `cargo audit`: `quinn-proto` 0.11.14 (RUSTSEC-2026-0185, remote memory exhaustion, fix ≥0.11.15, reached via reqwest ← vectorizer-sdk/synap-sdk/nexus-graph-sdk ← every cortex-* crate) and `rmcp` 0.8.5 (RUSTSEC-2026-0189, DNS rebinding, fix ≥1.4.0, reached via nexus-protocol ← nexus-graph-sdk), plus 3 lower-severity warnings (2 unmaintained deps, 1 unsound `anyhow` issue). Both advisories post-date the CHANGELOG's earlier "cargo audit green" claim — not a regression from this session's work. *Verified: direct `cargo audit` output.*

**F-008 — cortex-graph-worker has been silently stalled since 2026-06-27.** Its Nexus-consumer loop stopped logging any activity after a run of transient Nexus errors; Docker's HEALTHCHECK kept reporting the container "healthy" for the following 8 days because it only checks that `/healthz` responds, not that the consume loop progresses. cortex-api's own `/v1/health` freshness check (600-second no-activity threshold, `admin_health.rs`) correctly flags it "degraded" right now. *Verified: `docker logs cortex-graph-worker`, `/v1/health` live call.*

**F-009 — Operator doctor scripts are broken.** `bin/cortex-doctor` and `bin/cortex-doctor.ps1` both reference a nonexistent `cortex-ops` package; it's actually a `[[bin]]` target inside `cortex-cli` (`crates/cortex-cli/src/bin/cortex-ops.rs`). *Verified: file inspection, reproduced the failure.*

**F-010 — `cortex-ops doctor` fails 100% of checks on native Windows.** Once invoked correctly, the tool shells out to `curl -o /dev/null`, which fails with libcurl error 23 on native Windows (not WSL) because `/dev/null` isn't a real path there — producing false failures on every check even though the target services are healthy (independently disproven via direct curl calls and `doctor-config.sh` in the same session). The identical idiom works inside the project's own Linux container HEALTHCHECK lines, which is why this went unnoticed. *Verified: reproduced the failure, cross-checked against working direct curl calls.*

**F-011 — Local-stack spec undercounts the real stack.** `docs/specs/03-local-stack.md` describes far fewer services than the actual 12-service `docker-compose.yml`. *Verified: direct file comparison.*

## Retrieval quality axis

**F-012 — Retrieval eval gate has structural drift, not the placeholder-data problem originally assumed.** `.github/workflows/eval.yml`'s `--golden tests/golden/${{ matrix.suite }}.csv` has no `working-directory:` override, so it resolves against the repo-root `tests/golden/` tree, while the `cargo test` IT gate (`golden_set_acceptance.rs`) loads from `crates/cortex-eval/tests/golden/` via `CARGO_MANIFEST_DIR` — the two trees have diverged (different row content under the same ids; the root `mcp_search.csv` has all 16 `expected_ids` blank; the root tree has no `access_control.csv` at all). The `classification` suite has never actually run (`finished_at: 1970-01-01`, `rows_total: 0`); `access_control`/`mcp_search` suites exist in code but aren't in the CI matrix; phase17 P2's p95-latency arm and P3's phantom-link-rate metric were never measured. Re-enabling the nightly schedule without reconciling the two fixture trees first would gate CI on stale data. This remains the highest-leverage retrieval-observability fix, just a different root cause than initially assumed. *Verified: direct comparison of both fixture trees + workflow YAML read, corrected during task authoring.*

**F-013 — Embedding model imposes a hard cosine-similarity ceiling independent of pipeline hygiene.** nomic-embed-text (384-dim) caps raw cosine similarity at ~0.42–0.45 on a representative benchmark query; a ~90% corpus dedup/purge pass (phase26e) did not move this ceiling. *Verified: cross-referenced against the archived phase26e task's own measurements.*

**F-014 — Semantic graph projection is permanently disabled pending an upstream bug.** `CORTEX_GRAPH_PROJECTION_ENABLED=false` in `docker-compose.yml`, citing nexus#12 (a sustained-write stall in the upstream Nexus graph DB) — still open and undocumented as fixed anywhere, even though Nexus itself is already on 2.3.4 (which carries the phase25 sequential-MATCH mitigation and the edge-props-on-MERGE fix). Per `docs/analysis/graph/README.md`, unblocking this is estimated to lift 2-hop `pre_change_context` hit-rate from 28%→75% and decision-trail completeness from 10%→80%, and it blocks the already-written phase27b/c/e GraphRAG task chain from having any live value. *Verified: config/flag inspection, ADR-027 cross-reference.*

**F-015 — Turns still embed raw JSON in some paths.** phase26f §1 (clean-NL turn re-embed) remains blocked on an operator-authorized destructive re-embed window. *Verified: task-tracker cross-reference.*

## Agent-workflow axis

**F-016 — No runtime capability-discovery mechanism for the 37-tool MCP surface.** An agent connected to Cortex has no way to enumerate what the tools do without reading source or docs. *Verified: MCP schema inspection.*

**F-017 — Governance engine's blocking mechanism is already built client-side but never wired server-side.** Deeper research during task authoring found the adapter's `PreToolUse` path (`crates/cortex-adapter-claude-code/src/sync_paths.rs` + `dispatcher.rs`) already POSTs to `/v1/laws/check`, fails open, and computes `deny` from `severity == "critical"` with `block_on_critical` defaulting to `true` — the client-side blocking plumbing is complete. It silently no-ops today only because `cortex-api` has zero dependency on `cortex-laws` and no `/v1/laws/check` route exists server-side. This reframes the governance MVP as a surgical server-side wiring task rather than a from-scratch build — but also means shipping that route will flip live blocking on immediately for any `severity: critical` law, which needs care. *Verified: end-to-end code trace through both crates.*

**F-018 — Adapter coverage is Claude-Code-only in practice, but not for the reason first assumed.** Deeper research found `cortex-adapter-opencode` and `packages/cortex-opencode-plugin` are actually code-complete with green tests (shipped phase16a, 2026-06-10) — the earlier claim of "still a placeholder" was stale. What's actually true: live end-to-end validation was explicitly closed WON'T-DO in `phase16b_opencode-smoke-validation` (2026-06-22) by an operator decision to deprioritize OpenCode ("the operator works in Claude Code"). Cursor/Codex/Gemini adapters genuinely don't exist. *Verified: direct code read, corrected during task authoring after the initial brief proved stale.*

**F-019 — Feedback loop is half-wired, and the schema itself is a blocker.** `cortex_feedback_record`/`cortex_feedback_signals` exist, but `crates/cortex-api/src/search/strategies.rs` (the RRF fusion code) shows no evidence of reading them back. Deeper research found the `pre_thinking_feedback` SQLite schema is bundle-level, not lane-level — there's no column tying a helpful/unhelpful verdict to which lane (vector/keyword/graph) produced the hits, which any naive "just read the signal back" implementation would miss. *Verified: full read of `fusion.rs`, `strategies.rs`, and the metadata schema.*

## Observability axis

**F-020 — "Ship-then-dead-wire" is a confirmed recurring anti-pattern.** The phantom-link verifier landed dead-wired; pre-thinking cache counters were invisible cross-process; the adapter daemon was simply not running in an earlier incident; and now the graph-worker stall (F-008) found today. Captured as a durable knowledge-base entry this session.

**F-021 — Container-level and application-level health signals can disagree, and the application-level one is the one that caught the real problem.** Docker's HEALTHCHECK on cortex-graph-worker only probes `/healthz`; cortex-api's own freshness check (600s threshold) is what actually flagged the stall. Captured as a durable knowledge-base entry this session.

## Memory/continuity axis

**F-022 — The consolidation→next-session pre-thinking injection loop (the actual "/clear-amnesia" fix) has no end-to-end proof it works.** The pieces exist (consolidations, `cortex_session_replay`, `cortex_active_work`, handoffs view, `cortex_capture_memory`) but nothing currently proves a prior session's consolidation is injected into the very next session's bundle.

**F-023 — `cortex_active_work` isn't wired into session start.** A new session doesn't automatically see prior in-flight Rulebook work; the agent has to think to call the tool.

## Strategic re-scope

This analysis explicitly narrows scope to the coding-agent axis (retrieval, agent workflows, observability, continuity) per the user's explicit request, de-prioritizing the June 2026 analysis's drift toward a general business-KB platform (Person/Project/Case entities, Slack/Linear/Notion connectors, multi-tenant). See `docs/analysis/cortex/11-platform-vision-analysis.md` (marked superseded) for that earlier direction.

## Relationship to the task backlog

All 12 findings-derived roadmap items were materialized as Rulebook tasks (`phase28_live-testing-bugfixes` through `phase33_adapter-opencode-cursor`) during this same session. Several tasks' authoring passes did additional research that refined or corrected findings above (noted inline where it happened, e.g. F-012, F-017, F-018, F-019) — the task proposals are the more detailed, more current source for implementation; this document is the narrative audit trail of how the roadmap was derived.
