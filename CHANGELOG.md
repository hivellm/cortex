# Changelog

All notable changes to **Cortex** will be documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — 0.1.0-alpha

> First substantive cut of Cortex — workspace, ingestion bus, classifier worker, the four indexers, query API, dashboard backend + GUI, MCP server, and Claude Code plugin. Spec coverage: **13/18 🟢**, **5/18 🟡**.

### Added

#### Foundation (Phase 0)
- **Spec 01 — Event schema (wire format)** ([`cortex-core`](crates/cortex-core/)) — typed envelope, validator, redactor.
- **Spec 02 — Storage layout** ([`cortex-storage`](crates/cortex-storage/)) — locked layout for every backend (Vectorizer collections, Nexus labels, Meili indices, Synap streams).
- **Spec 03 — Local stack** — `docker-compose.yml` + `bin/cortex-{up,down,reset,doctor,logs}` wrappers + ops CLI (`cortex-ops plan|doctor|smoke`).
- **Spec 04 — Cortex Core ingestion router** — redactor, ingestion HTTP service, canonical envelope batch shape (`{events: [...]}`).

#### Capture & classify (Phase 1)
- **Spec 05 — Classifier** ([`cortex-classifier`](crates/cortex-classifier/) + [`cortex-classifier-worker`](crates/cortex-classifier-worker/)) — Claude Haiku via Claude Code CLI 2.x (parses `result` field, no `--max-tokens`); SDK path optional. Worker isolated in its own crate to break the embedder ↔ classifier cycle ([ADR-002](.rulebook/decisions/002-classifier-worker-lives-in-a-separate-crate-to-avoid-the-classifier-embedder-classifier-cycle.md)).
- **Spec 06 — Embedder** ([`cortex-embedder`](crates/cortex-embedder/)) — Tree-sitter symbol chunking for code, section chunking for docs, Vectorizer client, worker loop. JWT auth + 512-dim alignment + Meili URL env wiring.
- **Spec 07 — Graph writer** ([`cortex-graph`](crates/cortex-graph/)) — Cypher template registry, per-kind payload expansion, parent-anchored edges, worker loop with Synap I/O + out-of-order buffer, per-row Cypher renderers, ASCII-only escape helper, `assert_write_landed`, `OBSERVED_IN` on canonical kind discriminator. Backfill bin.
- **Spec 08 — Full-text indexer** ([`cortex-fulltext`](crates/cortex-fulltext/)) — Meilisearch client, agent_call routing to turns, artifact routing by path + topics, `routed_total` metric, `primaryKey=id` upsert, fan-out parity.
- **Spec 09 — Bootstrap CLI** ([`cortex-bootstrap`](crates/cortex-bootstrap/)) — walks any Hive repo, emits envelope-compliant events on `cortex.events.bootstrap`. Per-repo `cortex.toml` (excludes, chunking, git history). Default `.rulebook/*` discovery across all repos. Per-event publish failures tolerated (5% / 20-floor floor). One Session per bootstrap run; `.rulebook/specs/*.md` split into per-section law envelopes.

#### Adapters (Phase 1)
- **Spec 10 — Claude Code adapter** ([`cortex-adapter-claude-code`](crates/cortex-adapter-claude-code/)) — local daemon + hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, `Notification`). Stop hook stamps assistant-message Turn envelope. Real `tool_input` capture, multi-session distinction, pre-thinking pipeline wiring with camelCase hook contract.
- **Spec 18 — Claude Code plugin** ([`cortex-plugin/`](cortex-plugin/) + [`cortex-mcp-server`](crates/cortex-mcp-server/)) — stdio MCP bridge with 3 tools (`cortexQuery`, `cortexPreThinking`, `cortexStatus`); 6 slash commands; 3 skills; 3 sub-agents; Windows hook compatibility; sync-cache helper.

#### Retrieval (Phase 1)
- **Spec 11 — Query API** ([`cortex-api`](crates/cortex-api/)) — `POST /v1/query` hybrid retrieval (Vectorizer + Meilisearch + Nexus) + Reciprocal Rank Fusion. Slug-aware cache invalidation, canonical scope echo, source-attribution invariant. Live Nexus-backed `GraphLane`, live Vectorizer-backed `VectorLane` (SDK 3.0.3), live Meilisearch keyword lane. Per-project collection / index isolation. Periodic archive re-scan keeps the lane fresh.
- **Spec 12 — Pre-thinking injection** ([`cortex-pre-thinking`](crates/cortex-pre-thinking/)) — library that turns the `/v1/query` bundle into a deterministic Markdown system-prompt block under a byte budget.

#### Dashboard (Phase 2 — backends shipped, polish ongoing)
- **`/v1/dashboard/*` endpoints** — overview, timeline (recent + SSE stream with reconnect ladder), memory, decisions (+ detail), laws, violations, analyses, tools/stats, graph (edge-first endpoint, name labels, timeline dedup), sessions, trust, conversations (+ session detail with Sonnet-backed cross-event analyzer), handoffs, **rulebook tasks** (`/v1/dashboard/tasks*`).
- **Meili loader** — unblocks decisions / laws / memory dashboards; law catalogue derived from `law_violation` envelopes.
- **GUI (`gui/`)** — React + TypeScript dashboard + Electron shell.
  - **phase2a** — sparkline smoke wire + Atoms README section.
  - **phase2b** — Timeline stats grid + stream controls.
  - **phase2c** — Inspector richer.
  - **phase2d** — Decisions register + Law dashboard polish.
  - **phase2e** — Tweaks drawer (theme / accent / density / sidebar).
  - **phase2f** — Live SSE Timeline + reconnect ladder.
  - **phase2g/phase2h** — Enriched metrics, Decision-chain + graph richness backend.
  - **Graph explorer** — Cytoscape renderer, structural view, project palette, drill-down, label promotion, Session-to-Repo hue inheritance.
  - **Conversations / Handoffs / Analyses** — per-project decision filter; analyses promoted as first-class envelopes; full-width drawer on narrow viewports + horizontal-scroll lock.
  - **Classifications view** — surfaces classifier output corpus-wide.
  - **Sessions sidebar** + session / repo / kind filters end-to-end.
- **Auth** — phase2f dashboard auth tasks tracked.

#### Operations
- **Workspace ports migrated** from `1500x` → `1700x` to avoid conflicts with stand-alone Vectorizer / Synap installations.
- **Doctor + smoke targets** — `make doctor` health-probes every backend; `make smoke` runs `up + doctor + plan`.

#### Observability — pipeline stage metrics & freshness (phase8b)
- **Per-stage divergence counters across the whole pipeline.** Each stage now exports per-kind / per-hook counters via `/healthz` extras *and* a Prometheus-text `/metrics` endpoint mounted on the same listener:
  - `cortex-adapter-claude-code` — `frames_received_total{hook}`, `frames_parsed_total{hook}`, `frames_parse_error_total`, `envelopes_built_total{kind}`, `envelopes_publish_ok_total{kind}`, `envelopes_publish_fail_total{kind}`, `last_frame_ts_ms{hook}`, `last_envelope_ts_ms{kind}`, `last_publish_ok_ts_ms{kind}`.
  - `cortex-ingestion` — `events_received_total{kind}`, `events_archived_total{kind}`, `events_rejected_total{reason}`, `last_archive_write_ts_ms{kind}`.
  - `cortex-api` — `archive_loader_envelopes_seeded_total{kind}`, `meili_loader_docs_seeded_total{family}`, `*_last_refresh_ts_ms`.
  - Workers — `*_jobs_processed_total`, `*_last_job_ts_ms` per worker.
- **NEW `GET /v1/health/freshness`** on `cortex-api` — fans out the same probes `/v1/health` uses, parses the per-stage `last_*_ts` extras, and returns a flat array of `{ key, last_event_ts_ms, gap_seconds, severity }` rows keyed `<stage>.<kind>`. Severity buckets: `gap_seconds > 60` → `warn`, `> 300` → `critical`.
- **NEW `GET /v1/health/divergence`** on `cortex-api` — pairs adjacent-stage counters (`adapter.frames_parsed → adapter.envelopes_built → adapter.envelopes_publish_ok → ingestion.events_archived`) and reports `{ pair, upstream, downstream, delta, delta_growth, severity }` rows. The aggregator caches the previous probe's delta in process memory so `delta_growth > 0` localises a silent drop to the offending boundary in seconds.
- **`cortex-health::server` now mounts an optional `/metrics` Prometheus-text endpoint** alongside `/healthz` (`serve_standalone_with_metrics` / `router_with_metrics`). Workers and the adapter wire it through `spawn_health_listener_with_metrics`.
- Closes the 2026-04-28 JSON-truncation incident class: a divergence row of shape `adapter.frames_parsed → adapter.envelopes_built ≈ N delta_growth` would have fired within seconds instead of the ~2 h grep-the-logs trace that actually found it.
- Canonical metric catalogue: [`docs/metrics.md`](docs/metrics.md). Architecture writeup: [`docs/architecture.md` §13.6](docs/architecture.md#136-observability--pipeline-stage-metrics--freshness-phase8b).

#### Observability — version coherence (phase8c)
- **NEW [`cortex-build`](crates/cortex-build/) crate** — workspace-shared build-time emitter. `emit_version_env()` from each crate's `build.rs` stamps `CORTEX_GIT_SHA` / `CORTEX_GIT_SHA_SHORT` / `CORTEX_BUILD_TS` / `CORTEX_GIT_DIRTY` / `CORTEX_BUILD_PROFILE` via `cargo:rustc-env`; the runtime `version_info!()` macro reads them back as a `VersionInfo` struct.
- **Every binary's `/healthz` extras carries a `version` block** — adapter, ingestion, cortex-api, and all four workers stamp `git_sha`, `build_ts`, `git_dirty`, `profile`, `crate_version` so the running binary's provenance is observable.
- **NEW `GET /v1/health/versions`** on cortex-api — fans out, parses `extras.version`, computes drift against workspace HEAD captured once at boot, returns `{ head_sha, running_binaries[], drift[], all_in_sync }`. `behind_by_commits` is computed via `git rev-list <running_sha>..HEAD --count` per drifted binary.
- **NEW operator scripts** — `scripts/doctor-versions.{bat,sh}` curl the endpoint, print a table, exit non-zero when `all_in_sync == false`.
- **NEW CI gate** — `.github/workflows/version-coherence.yml` rejects PRs whose committed `target/release/<bin>` mtime is older than the most-recent source mtime in the owning crate (defensive — the project doesn't normally commit `target/`).
- Closes the 2026-04-28 incident where the source had the fix but `cortex-api.exe` had been built before the commit and there was no way to ask the running daemon "what git SHA were you built from?".

#### Observability — config coherence (phase8d)
- **NEW `cortex_api::config_audit` module** — pure-function audit of every config surface (`.env`, `~/.cortex/adapter.toml`, `cortex-plugin/.mcp.json`, `cortex-plugin/hooks/hooks.json`) plus cross-checks (e.g. `adapter.toml.endpoint` MUST equal `.env CORTEX_INGESTION_URL`). Each surface has its own typed reader (`read_env_file`, `read_adapter_toml`, `read_mcp_json`, `read_hooks_json`) returning `ReadError::NotFound`/`ReadError::Parse` instead of panicking.
- **NEW `GET /v1/health/config` endpoint** on `cortex-api` — runs the audit server-side via `spawn_blocking`, returns the `ConfigAudit { findings[], surfaces_read }` JSON. The dashboard renders findings as a table; CI curls and gates on `worst_severity`.
- **NEW `cortex-ops doctor-config` subcommand** + `scripts/doctor-config.{bat,sh}` — run the same audit locally. Exit codes: `0` all ok, `1` any warn, `2` any critical. Supports `--json` for machine-readable output and `--workspace` / `--adapter-toml` overrides for fixtures.
- Closes the 2026-04-28 incident's first wrong turn: the adapter was talking to `:15010` while ingestion was bound to `:17010`, the config file had the right value, but a stale daemon was holding the old endpoint in memory. The audit names the discrepancy as `severity: critical` with a single-line message containing both ports.

### Decisions
- **[ADR-001](.rulebook/decisions/001-bypass-vectorizer-sdk-for-insert-and-get-vector-direct-reqwest-until-sdk-server-drift-is-resolved.md)** — Bypass Vectorizer SDK for `insert` + `get_vector`, use direct `reqwest` until the SDK / server drift is resolved.
- **[ADR-002](.rulebook/decisions/002-classifier-worker-lives-in-a-separate-crate-to-avoid-the-classifier-embedder-classifier-cycle.md)** — Classifier worker lives in a separate crate to avoid the classifier ↔ embedder cycle.

### Fixed
- **`cortex-fulltext`** — route `agent_call` to turns instead of dumping in docs; route artifacts by path + topics; `primaryKey=id` on upsert; spec-05 summary contract.
- **`cortex-graph`** — surface silently-dropped edges via `assert_write_landed`; stamp `label/display/caption/name` on every node.
- **`cortex-classifier`** — parse `result` field from Claude Code CLI 2.x; drop `--max-tokens` for CLI 2.x.
- **`cortex-plugin`** — make hook capture work on Windows; `hooks.json` location inside `hooks/`; marketplace source at plugin root; document directory-source cache pitfall + ship `sync-cache.sh`.
- **`cortex-adapter-claude-code`** — wire pre-thinking pipeline with camelCase hook contract; emit canonical spec-04 envelopes.
- **`cortex-ingestion`** — accept spec-04 `{events:[...]}` batch shape.
- **`cortex-mcp-server`** — rename tools to identifier-safe names + camelCase schema fields (spec-18 compliance).
- **GUI** — drawer covers full width on narrow viewports + lock horizontal scroll.

### Pending (🟡 specs in progress)
- **Spec 13** — Laws DSL + detector contract.
- **Spec 14** — Governance engine (enforcement, punishment, trust score).
- **Spec 15** — Deep Analysis workflow.
- **Spec 16** — Dashboard polish (views shipped, refinement ongoing).
- **Spec 17** — Cursor / Codex / Gemini adapters.

---

[Unreleased]: https://github.com/hivellm/cortex/compare/HEAD
