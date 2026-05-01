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

#### Observability — silent-drop detector (phase8e)
- **NEW background watcher in `cortex-api`** (`crates/cortex-api/src/silent_drop.rs`) — polls the same divergence pairs `/v1/health/divergence` surfaces, runs each pair through a debounced state machine (`Ok → Warn` requires 2 consecutive polls > `warn_delta`; `Warn → Critical` fires on 1 poll > `critical_delta`; recovery requires 5 consecutive non-growing polls), and emits `law_violation` envelopes on every transition.
- **Push channel** — each alert envelope lands in the durable archive (POSTed to cortex-ingestion `/v1/events/batch`) AND the in-memory `MemoryKeywordLane` so the alert shows up in the existing Live Timeline + Violations dashboard within ~1s, no manual endpoint poll needed.
- **Per-pair state persists** at `~/.cortex/alerts/<pair>.json` so a daemon restart does not re-fire alerts the previous run already flagged.
- **Optional escalation hooks** — `SilentDropConfig.webhook_url` POSTs the envelope to a webhook on every transition; `write_to_handoff: true` appends a `[silent-drop alert]`-prefixed line to `.rulebook/handoff/_pending.md` on every Critical transition so the next session inherits the context.
- Closes the 2026-04-28 silent-drop incident verbatim: the divergence between `adapter.frames_parsed` and `adapter.envelopes_built` would surface as a `law_violation` envelope within ~60s of the truncation hitting, instead of the ~2h log-grep search that actually found it.

#### Observability — synthetic E2E canary (phase8f)
- **NEW `cortex_api::canary` module** — fires a synthetic hook frame through the real IPC pipe (`\\.\pipe\cortex-adapter-claude` on Windows, `~/.cortex/adapter-claude.sock` on Unix) and polls `/v1/dashboard/timeline/recent` for the embedded marker until a configurable deadline elapses. Frame body deliberately mimics the 2026-04-28 regression vector: pretty-printed JSON with multi-line `\n`-escaped strings inside `tool_response.stdout`.
- **NEW `cortex-ops canary` subcommand** + `scripts/canary.{bat,sh}` — exit codes `0` round-trip success, `1` transport error, `2` deadline timeout. Suitable for ad-hoc smoke checks and CI gates.
- **NEW `cortex-api` background runner** — opt-in via `CORTEX_CANARY_ENABLED=1`. Ticks every `CORTEX_CANARY_INTERVAL_SECS` (default 300), appends every result to `~/.cortex/canary-history.jsonl`, and POSTs a `law_violation` envelope (severity `critical`, `law_id: canary-<hook>`) on failure via the same path phase8e uses — so quiet-hours regressions surface in the existing Violations dashboard automatically.
- Closes the quiet-hours failure class: a regression like the 2026-04-28 JSON truncation is now detected in 10s ad-hoc or ~5 minutes scheduled instead of hours.

#### Observability — dashboard Health view (phase8g)
- **NEW `gui/src/views/Health.tsx`** — first-class Health dashboard surfacing the phase8a–8f system: overall banner, subsystems grid, freshness table sorted by `gap_seconds` desc, divergence table filtered to `severity != ok`, version-drift section (rendered only when `all_in_sync == false`), config audit. Each section degrades gracefully to an empty-state when its endpoint times out.
- **NEW `GET /v1/health/stream` SSE endpoint** on cortex-api — emits a combined `HealthSnapshot { overall, freshness, divergence, truncated }` every 5 s plus a `heartbeat` every 15 s. Snapshot byte-capped at 64 KiB; oversized payloads halve the freshness vec and flip `truncated: true` so the GUI can render an explicit "showing N of M" hint.
- **NEW topbar status pill** in the GUI header — visible from every view (Live Timeline, Conversations, Decisions, …) with green/yellow/red dot driven by `/v1/health.overall`. Click jumps to `/health`. Polls every 5 s; the user can't miss a stack-degraded state while browsing.
- **NEW typed API clients** in `gui/src/lib/api.ts` (`healthOverview`, `healthFreshness`, `healthDivergence`, `healthVersions`, `healthConfig`) + matching TypeScript types (`HealthOverview`, `FreshnessRow`, `DivergenceRow`, `VersionsReport`, `ConfigAudit`, `HealthSnapshot`).
- 5 new vitest tests in `gui/src/views/Health.test.tsx` covering banner rendering, subsystem cards, divergence-row visibility, config-audit ok-row filtering, freshness gap-label format. `pnpm test` reports 15/15 passing.

#### Observability — CI smoke gate (phase8h)
- **NEW `.github/workflows/health-smoke.yml`** — runs on every PR + push to main, matrix `[ubuntu-latest, windows-latest]`, 12-minute budget. Boots the full stack (cortex-ingestion + cortex-api + cortex-adapter-claude-code), polls `/v1/health`, then runs `health` + `doctor-versions` + `doctor-config` + `cortex-ops canary` in series. Any non-clean exit fails the PR. Failure path uploads `$CORTEX_HOME/logs/*.log` as a named artifact for postmortem.
- **NEW `scripts/ci/boot-stack.{sh,bat}` + `teardown-stack.{sh,bat}`** — reusable boot helpers. Honour `CORTEX_HOME` for concurrent-run isolation. `boot-stack` waits for `/v1/health.overall` to reach `ok` or `degraded` (60 s timeout); `teardown-stack` reads `$CORTEX_PIDS_FILE` and SIGTERM (then SIGKILL after 5 s) every spawned daemon.
- **NEW `.github/PULL_REQUEST_TEMPLATE.md`** — adds a "Health checks" section with checkboxes for `scripts/health`, `scripts/doctor-versions`, `scripts/doctor-config`, `scripts/canary` outcomes. Soft cultural signal that complements the automated workflow gate.
- Closes the regression-reaches-main failure class: a bug like the 2026-04-28 JSON truncation cannot merge because the canary in CI would round-trip a multi-line `\n`-escaped frame and observe the missing envelope.

#### Storage — retention sweeper core (phase9a)
- **NEW [`cortex-retention`](crates/cortex-retention/) crate** — `SweepPlan` + `TierPair` + `VectorizerOps` trait + `run_sweep` implement spec 02's tier-transition contract: FP32 → PQ at 30 d, PQ → Binary at 365 d for `turn` / `tool_call` / `code_chunk`. Always-hot kinds (`decision`, `analysis`, `memory`, `law`) are deliberately absent. `MemoryVectorizerOps` is the in-memory test double that drives every spec scenario without a live Vectorizer.
- **NEW `cortex-ops retention-sweep` subcommand** + `scripts/retention-sweep.{bat,sh}` — `--time-travel <RFC3339>`, `--dry-run`, `--batch-size N`, `--metadata-db PATH`, `--json`. Exit codes: `0` success, `1` error-rate ceiling tripped / hard failure, `2` another sweep in flight.
- **`retention_sweeps` SQLite bookkeeping** — extended with a `status` column (`running` / `success` / `failed` / `abandoned`) plus three `MetadataStore` helpers: `start_retention_sweep` (concurrency lock + abandonment grace), `finish_retention_sweep`, `list_recent_sweeps`. The migration is idempotent — pre-phase9a databases get the column added via `ALTER TABLE` without bumping `SCHEMA_VERSION`.
- **Idempotent + crash-safe** — re-encode → upsert dest → delete source. A re-run short-circuits on `dest_has(event_id)`. A mid-flight crash leaves the source record alone; the next sweep finds the destination row already in place and cleans the source. The 5 % `max_error_rate` ceiling fails the run but still writes the bookkeeping row so the dashboard surfaces the regression.
- **Spec doc**: NEW [`docs/specs/19-retention.md`](docs/specs/19-retention.md) (wire shape + exit-code map + idempotence + error budget + observability contract).
- **Tests**: 16 unit tests in `cortex-retention/src/lib.rs` (every spec scenario verbatim — FP32→PQ at 31 d, PQ→Binary at 366 d, fresh-record no-op, idempotent re-run, dry-run observe-only, ceiling-trip vs ceiling-allow drop-rate); 6 unit tests in `cortex-storage/src/metadata.rs` (concurrency lock, abandonment grace, finish counters, list-newest-first, honours-limit). `cargo test --workspace` 0 failures.

#### Storage — archive rollup compactor (phase9b)
- **NEW `cortex_retention::parquet_rollup` module** — implements spec 02 §"Event archive (Parquet)" rollup contract: hourly → daily at 90 d, daily → monthly at 365 d, drop monthly at 3 y unless `kind ∈ {decision, analysis, law_violation}` or `redactions[].pii_risk = "low"`. The on-disk format is zstd-compressed NDJSON (despite the `.parquet` suffix); the compactor concatenates source rows line-by-line.
- **Atomic + crash-safe** — read → write `<dest>.tmp` → `sync_all` → `rename` → `unlink sources`. Row-count assertion (`sources_rows == dest_rows`) catches bad merges before commit. Orphan `.tmp` from a previous crash gets cleaned up on entry.
- **Corruption quarantine** — `*.corrupted*`, orphan `*.tmp`, and any file that fails zstd decode get moved to `events/_quarantine/<relpath>` with a sibling `.reason` text file. Query layer skips paths under `_quarantine/` automatically.
- **NEW `cortex-ops rollup` subcommand** + `scripts/rollup.{bat,sh}` — `--time-travel`, `--dry-run`, `--granularity all|hourly-to-daily|daily-to-monthly|three-year-drop`, `--archive-root`, `--json`. Default exit `0` even when individual partitions fail; `1` on hard error.
- **Spec doc**: `docs/specs/19-retention.md` extended with the rollup contract (granularities table, atomicity protocol, quarantine layout, whitelist semantics, RollupCounts shape).
- **Tests**: 11 new unit tests in `parquet_rollup.rs` covering every spec scenario verbatim — 91-day-old hourly directory becomes a daily file, 366-day-old daily files merge into a monthly file, 1100-day decision survives the 3-y drop while a same-age plain turn does not, monthly file gets removed outright when no record passes the whitelist, `*.corrupted*` + orphan `*.tmp` quarantine on entry, granularity serde round-trip. `cargo test --workspace` 0 failures (27 cortex-retention tests total).

#### Storage — CAS vacuum (phase9c)
- **NEW `cortex_retention::cas_vacuum` module** — implements spec 02 §CAS weekly vacuum: deletes rows where `refcount = 0 AND last_referenced < now - 30 d`, then `VACUUM`s the metadata DB when the freelist exceeds 25 % of pages. Per-batch transactions (`BEGIN IMMEDIATE`, 256 rows) keep `SQLITE_BUSY` away from concurrent ingestion. The `DELETE … WHERE refcount = 0` predicate guards against TOCTOU when a concurrent `retain` lands mid-batch.
- **Catastrophic-deletion safeguard** — refuses when the candidate set exceeds 50 % of total blobs unless `--force`; a dry-run surfaces `safeguard_tripped: true` instead of erroring so operators can preview the problem without abort.
- **NEW storage helpers** in `cortex_storage::cas`: `select_vacuumable(cutoff, limit) -> Vec<VacuumCandidate>`, `delete_blobs(&[hash])` (per-batch tx with `BEGIN IMMEDIATE`), `total_blob_count`, `total_blob_bytes`, `conn_mut`. New `VacuumCandidate { hash, size }` row.
- **Refcount audit** — `audit_refcounts(store, references)` recomputes the expected refcount from a caller-supplied iterator of external references and returns `Vec<RefcountDrift { hash, claimed, observed }>`. `fix_refcounts(store, drift)` writes the observed values back in one `BEGIN IMMEDIATE` transaction.
- **NEW `cortex-ops cas-vacuum` subcommand** + `scripts/cas-vacuum.{bat,sh}` — `--time-travel`, `--dry-run`, `--force`, `--cas-db PATH`, `--json`. Plain-text summary surfaces `total_blobs`, `dropped`, `bytes_reclaimed`, `free_pages_ratio`, `did_vacuum`, `vacuum_ms`, plus a WARN line when the safeguard would trip.
- **Spec doc**: `docs/specs/19-retention.md` extended with the CAS vacuum contract (eligibility, atomicity, reclamation, safeguard, refcount audit, test surface).
- **Tests**: 13 new unit tests covering every spec scenario verbatim — 31-day orphan deleted, fresh blob preserved, dry-run no-op, safeguard refuses + force overrides + safeguard no-op on empty store, audit reports under/over-count drift + aligned no-drift, fix_refcounts writes observed, batches split evenly. `cargo test --workspace` 0 failures (40 cortex-retention tests total).

#### Storage — PII retention enforcement (phase9d)
- **NEW `cortex_retention::pii_enforce` module** — implements spec 01 §PII tiers. `classify(plan, target)` maps each candidate to one of `High30d` / `Medium90d` / `NullSafety90d` (or `None` for fresh / already-redacted / pii_risk=low rows). `run_enforcement(plan, backend, targets)` walks the cohorts and dispatches to the `PiiBackend` trait in the spec-mandated cross-store order — Parquet → Vectorizer → Meili → CAS for the high path, summarize → re-embed → re-index → Parquet → CAS for the medium / null-safety paths.
- **Null-safety net** — records with `pii_risk = null` AND age ≥ 90 d enter the medium path automatically and emit a `cortex.warnings` event so classifier-coverage gaps are auditable.
- **Idempotent re-runs** — `classify` filters out rows whose `payload.redacted` is already set; a partial run that crashes mid-flight rolls forward on the next sweep without double-summarizing.
- **`PiiBackend` trait surface** — minimal eight-method abstraction (`rewrite_row`, `delete_vector`, `delete_meili`, `decrement_cas`, `summarize`, `reembed_and_upsert`, `reindex_meili`, `emit_warning`) so production wires the live Vectorizer / Meili / CAS / classifier clients while tests use `MemoryPiiBackend` for in-memory round-trips with one-shot failure injection.
- **NEW `cortex-ops pii-enforce` subcommand** + `scripts/pii-enforce.{bat,sh}` — synthetic cohort preview today (built-in target suite covers each cohort + fresh no-op + already-redacted idempotence). Production-backend wiring lands with phase9k's cron scheduler.
- **Spec doc**: `docs/specs/19-retention.md` extended with the PII enforcement contract (cohort matrix, cross-store ordering, `PiiBackend` surface, CLI shape, test manifest).
- **Tests**: 16 new unit tests covering every spec scenario verbatim — classify high/medium/null/low/under-threshold/already-redacted, cohort redaction tags, high-path Parquet→Vector→Meili→CAS ordering, medium-path summarize+re-embed+re-index, null-safety warning + medium dispatch, dry-run no-mutation, cohort filter skips others, already-redacted skip, mid-flight failure recorded, cohort-counts serde. `cargo test --workspace` 0 failures (56 cortex-retention tests total).

#### Storage — LLM turn digest summarizer (phase9e)
- **NEW `cortex_retention::turn_digest` module** — bucketizes turns whose `occurred_at < now - digest_after_days` (default 30) by `(repo, ISO_year_week, top_topic)` and produces one Sonnet-driven `:Memory{memory_type="turn_digest"}` per non-empty bucket whose size ≥ `min_bucket_size` (default 5). The pure-function `bucketize` returns the deterministic plan; `run_turn_digest` orchestrates the per-bucket calls through the `DigestBackend` trait. ISO week labels follow RFC 8601 (`YYYY-Www`) so bucket keys are timezone-stable.
- **Idempotence** — `lookup_existing(repo, year_week, top_topic)` short-circuits already-digested buckets; `--rebuild` re-summarizes in place. A re-run with no new old turns reports `buckets_done=0, usd_cents=0`.
- **Cost ceiling** — `max_usd_cents_per_run` (default 500) caps per-run spend; the orchestrator stops cleanly when the running total breaches the budget and surfaces `buckets_pending` in the report so the next run resumes from the same point.
- **`DigestBackend` trait surface** — minimal four-method contract (`lookup_existing`, `summarize`, `persist_digest`, `tag_source_turns`). `persist_digest` does the full write fan-out (event emit + embed + Nexus `:Memory` + `[:SUMMARIZES]` edges) and returns the digest event id; `tag_source_turns` stamps `payload.summarized_by` on every source turn's Parquet row. Production wires the live classifier + embedder + Nexus writer + Parquet rewriter; tests use `MemoryDigestBackend` for in-memory round-trips with one-shot failure injection.
- **NEW `cortex-ops turn-digest` subcommand** + `scripts/turn-digest.{bat,sh}` — synthetic preview today (16-target suite covering two topics in one week). Production walker (Parquet + classifier + embedder + Nexus + Parquet rewriter integration) lands with phase9k's cron scheduler.
- **Spec doc**: `docs/specs/19-retention.md` extended with the LLM turn digest contract (cohort matrix, bucket key shape, `DigestBackend` surface, cost ceiling, CLI shape, test manifest).
- **Tests**: 14 new unit tests covering every spec scenario verbatim — `iso_year_week` label, bucket key format, bucketize groups by repo/week/topic, filters under-size, excludes fresh + already-digested, run persists one per bucket in call order, idempotent re-run, `--rebuild` re-summarises, `--dry-run` no-mutation, budget ceiling cuts cleanly, per-bucket failure recorded, JSON round-trip, plan defaults match spec. `cargo test --workspace` 0 failures (70 cortex-retention tests total).

#### Storage — Meilisearch archival pruner (phase9f)
- **NEW `cortex_retention::meili_prune` module** — implements spec 02 §Meilisearch retention. Documents in `cortex_turns` / `cortex_tool_calls` whose `occurred_at < now - prune_after_days` (default 90) get their body fields blanked + `summary` capped to 4 KiB with a UTF-8 ellipsis marker + `pruned: true` + `pruned_at` stamped. Documents are **never** deleted — the keyword lane still surfaces pruned rows on summary matches.
- **`cap_summary` UTF-8 safety** — when truncating, the cap walker rounds back to the nearest char boundary so multibyte codepoints never get sliced mid-character. When the cap is ≤ 3 bytes the helper returns the ellipsis verbatim.
- **`MeiliBackend` trait surface** — minimal two-method contract (`enumerate_prunable`, `update_documents`); deliberately omits `delete_documents` since pruning never deletes. Production wires the live Meili SDK; tests use `MemoryMeiliBackend` for in-memory round-trips with one-shot failure injection + `commit_updates` helper for the second-run idempotence test.
- **Idempotent re-runs** — pruned docs carry `pruned: true`, the matcher excludes them by default; `--rebuild` re-prunes when the cap policy changes.
- **Batched writes** — `update_documents` is called in chunks of `batch_size` (default 1 000) so a flaky task-await doesn't lose the whole run.
- **NEW `cortex-ops meili-prune` subcommand** + `scripts/meili-prune.{bat,sh}` — synthetic preview today (3 turns + 1 fresh + 1 oversize across both indexes). Production wiring lands with phase9k's cron scheduler.
- **Spec doc**: `docs/specs/19-retention.md` extended with the Meili pruning contract.
- **Tests**: 16 new unit tests covering every spec scenario verbatim — plan defaults, cap unchanged/truncated/char-boundary/ellipsis-edge, enumerate excludes fresh + already-pruned, runs prune across both indexes, second-run no-op, `--rebuild` re-prunes, dry-run no-mutation, oversize cap with ellipsis, failure propagation on enumerate + update, JSON round-trip, batch chunks split large runs. `cargo test --workspace` 0 failures (86 cortex-retention tests total).

#### Conversation archive + relevance tuning (phase11i)
- **NEW [`cortex-claude-archive`](crates/cortex-claude-archive/) crate** — walks Claude Code's `~/.claude/projects/<project>/*.jsonl` archive, parses every record, projects them into canonical Cortex envelopes, and ships them through one of two sinks (`stdout` or zstd-NDJSON `archive` partitions under `<archive_root>/events/year=…/…/bootstrap-claude-NNNNN.parquet` — re-read at boot by `cortex-api`'s `archive_loader`). Three subcommands: `estimate`, `bootstrap` (with `--resume` against a JSON checkpoint), and §5.2's `tail` watcher daemon.
- **`cortex-claude-archive tail` watcher daemon** — long-running poller (1 Hz, override via `CORTEX_CLAUDE_ARCHIVE_POLL_MS`) that re-reads any session whose `(mtime, len)` advanced and emits envelopes whose `event_id` is not in the in-memory dedupe set. Axum HTTP server on `:17030` (override via `CORTEX_CLAUDE_ARCHIVE_BIND`) exposes `GET /healthz` returning `last_flush_ts`, `files_watched`, `envelope_rate`, `rss_bytes`, `envelopes_emitted`, `envelopes_dropped`, `uptime_ms`, `status`. Cross-platform RSS via `sysinfo`; SIGTERM / SIGINT trigger graceful shutdown via a tokio `watch` channel.
- **`cortex-api` relevance fusion** — `FusionConfig` now carries phase11i §3.x knobs: per-intent recency-decay λ (`pre_change_context` / `similar_problems` / `free_search` / `explain` 0.02 ≈ 35-day half-life; `decision_lookup` 0.005 ≈ 140-day; `law_check` 0 evergreen), cross-repo boost (clamped `[0, 1]`), session cohesion (same-session ×2.0, cohort ×1.5), outcome filter + multipliers (success ×1.2, error ×0.5, blocked_by_law ×0.3). `Scope` request-side fields (`recency_decay`, `cross_repo_boost`, `models`, `tools`, `session_id`, `session_cohort`, `outcomes`, `exclude_outcomes`) round-trip through `canonicalise_scope`. Meilisearch `settings.v1.json` bumped to v3 with `model` / `tool` / `session_id` / `outcome` filterable.
- **NEW [`crates/cortex-api/config/relevance.toml`](crates/cortex-api/config/relevance.toml)** — boot-loaded + SIGHUP-reloadable knob file carrying every multiplier (RRF α/k, cross-repo boost, session boosts, outcome multipliers, per-intent recency λ). Orchestrator's `fusion` field becomes `Arc<RwLock<FusionConfig>>` so the SIGHUP-driven `Orchestrator::replace_fusion` path makes the swap visible to in-flight handles. Windows builds log a one-shot WARN — file still loads at boot.
- **Pre-thinking renderer (spec 12) refresh** — new `Past sessions` section between `Similar past turns` and `Relevant snippets` (top-3 by centroid similarity, first-prompt clipped to 80 bytes). Outcome glyph (`✓` / `✗` / `⚠`) on every turn + decision line driven by `cortex_api::SimilarTurn.outcome` + `DecisionRef.status`. Spec doc 12 §Output refreshed with both contracts.
- **NEW [`crates/cortex-api/tests/fixtures/relevance-gold.json`](crates/cortex-api/tests/fixtures/relevance-gold.json)** + **`relevance_eval_it.rs`** — 30-question hand-curated gold set (10 pre_change_context · 5 decision_lookup · 10 similar_problems · 3 law_check · 2 free_search). Live IT gated behind `CORTEX_RELEVANCE_IT=1` hits `cortex-api`, computes recall@10 / MRR@10 / NDCG@10 + per-intent MRR, panics when MRR@10 < 0.75. Scoring math runs unconditionally so a matcher regression fails the default `cargo test`.
- **NEW [`docs/cortex/relevance-tuning.md`](docs/cortex/relevance-tuning.md)** — operator handbook: gold-set authoring, IT runbook, reading the eval output, when to re-tune `relevance.toml` (and when *not* to — re-index, fix the gold entry), SIGHUP reload workflow, troubleshooting matrix.
- **`docker-compose.yml` `cortex-claude-archive` service** — read-only bind mount of the host's `~/.claude/projects` (override via `CORTEX_CLAUDE_PROJECTS_HOST`), shared `${CORTEX_HOME_HOST}` archive bind-mount with cortex-ingestion + cortex-api, depends_on synap (healthy) + cortex-ingestion (started), HEALTHCHECK on `:17030/healthz`. cortex-api env block defaults `CORTEX_ARCHIVE_WATCHER_URLS=http://cortex-claude-archive:17030` so `/v1/health/coverage` surfaces the watcher under a new `archive_watchers` block out of the box.
- **Tests**: 9 unit tests in `cortex-claude-archive::tail` (rate computation, snapshot shape, status transitions, axum router round-trip, env resolvers, clamp behaviour); §5.3 `memory_it.rs` (gated, 50 k pair / 100 k record corpus, asserts RSS < 512 MiB — measured 14.9 MiB on a real run); §5.4 `corrupt_line_it.rs` (corrupt lines counted in `envelopes_dropped`, valid records still emit, no panic, second-tick no-op); 8 unit tests in `cortex_api::relevance_config` + 5-case reload IT (boot read, `replace_fusion` propagation across cloned handles, missing file fallback, malformed TOML, intent-table round-trip); 8 `relevance_eval_it.rs` tests (matcher math + gated live path); 6 past-sessions formatter cases + 7 outcome-glyph cases. `cargo test --workspace` 0 failures.

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
- **`cortex-api` health routes (phase10g)** — `/v1/health`, `/v1/health/freshness`, `/v1/health/divergence`, `/v1/health/versions`, `/v1/health/config` mount on the dashboard-aware router; the audit caught the GUI's Health tab returning empty bodies because the running daemon predated the registration. Operators rolling forward from a pre-phase10g cortex-api MUST relaunch the binary; the `/v1/health/*` routes do not back-port to a process that is already serving requests.
  - Regression guard: [`every_v1_health_route_is_mounted_on_router_with_dashboard`](crates/cortex-api/tests/health_freshness.rs) + `cortex-ops doctor` probes the same URLs against `CORTEX_API_URL` so a missed registration in a future refactor surfaces in CI before it reaches a deploy.

### Pending (🟡 specs in progress)
- **Spec 13** — Laws DSL + detector contract.
- **Spec 14** — Governance engine (enforcement, punishment, trust score).
- **Spec 15** — Deep Analysis workflow.
- **Spec 16** — Dashboard polish (views shipped, refinement ongoing).
- **Spec 17** — Cursor / Codex / Gemini adapters.

---

[Unreleased]: https://github.com/hivellm/cortex/compare/HEAD
