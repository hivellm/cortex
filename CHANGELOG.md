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
