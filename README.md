# 🧠 Cortex

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org/)
[![Rust Edition](https://img.shields.io/badge/edition-2021-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/index.html)
[![Version](https://img.shields.io/badge/version-0.1.0--alpha-blue.svg)](CHANGELOG.md)
[![Tests](https://img.shields.io/badge/tests-3700%2B-brightgreen.svg)](crates/)
[![Status](https://img.shields.io/badge/status-alpha-yellow.svg)](docs/specs/00-index.md)

> **Governed, queryable memory for AI coding agents**

Cortex stores explicit decisions and verified lessons so coding agents can use them in later sessions. The daily profile keeps capture, cited recall and feedback in local SQLite, without mandatory Docker, GPU or bulk indexers. An optional full stack indexes code, documents and archived interactions.

## Daily Memory

For routine agent work, start with the [daily runtime guide](docs/operations/daily-memory-runtime.md). Capture an accepted decision or verified lesson with `cortex_capture_memory`; recover applicable context with `cortex_memory_read` or `cortex_pre_thinking`. Corrections explicitly supersede an earlier decision using the observed revision.

- Claude Code supports verified lifecycle delivery through its local adapter. Codex uses verified MCP reads; other harnesses have [documented capability limits](docs/analysis/daily-agent-memory-2026-10/09-client-contracts.md).
- Context carries source IDs and includes at most five memories within 1,536 bytes. Superseded, proposed and out-of-scope records stay out of ordinary advice.
- Capture remains local by default. Optional `classify=true` requests bounded Jev advice for an authorized candidate; classification never changes its status, scope or authority.
- Native agent memory files are preserved. Daily mode disables bulk publishing, archive replay and scheduled maintenance.
- [Behavioral acceptance](docs/analysis/daily-agent-memory-2026-10/11-cross-model-acceptance.md) verifies a Claude correction recalled and used by Codex after restart. [Resource measurements](docs/analysis/daily-agent-memory-2026-10/10-resource-acceptance.md) state the host, samples and limits.

## 📦 Installation

Cortex installs as a plugin for Claude Code or Codex. The plugin carries the hooks and the MCP entry, so no client file holds a Cortex path, token or environment block. Nothing is registered with the OS: the first session starts the local runtime ([ADR 040](.rulebook/decisions/040-app-managed-daily-runtime-lifecycle-started-by-cortex-hook-no-os-autostart.md)).

You need Rust 1.80+ and the Claude Code or Codex CLI. On Windows the hooks run under Git Bash, which Claude Code already requires.

### 1. Build and install the binaries

```bash
cargo build --release --bin cortex-api --bin cortex-adapter-claude --bin cortex-hook --bin cortex-mcp-server --bin cortex
target/release/cortex install --from target/release          # dry run: lists what would change
target/release/cortex install --from target/release --apply  # copies into ~/.cortex/bin
```

The installer verifies SHA-256 and moves a running executable aside before replacing it. Next, put `~/.cortex/bin` **first** on your PATH. Older copies elsewhere on PATH, such as `cargo install` binaries in `~/.cargo/bin`, would otherwise run first, and the hooks would use an outdated `cortex-hook`.

```powershell
# Windows (PowerShell), then open a new terminal
$p = [Environment]::GetEnvironmentVariable('Path', 'User')
[Environment]::SetEnvironmentVariable('Path', "$env:USERPROFILE\.cortex\bin;$p", 'User')
```

```bash
# Linux / macOS: add to your shell profile
export PATH="$HOME/.cortex/bin:$PATH"
```

`cortex` is installed too, so in a new terminal it runs by name. `cortex doctor --build target/release` should then report all five binaries as `ok`. It flags missing, shadowed or stale binaries.

### 2. Configure

```bash
cortex setup           # dry run
cortex setup --apply   # writes ~/.cortex/config.toml
```

The file selects the daily profile. To migrate an existing runtime file, pass `--from <runtime.toml>`. Its `dashboard.api_token` moves to `~/.cortex/api-token`, which only your user can read. Changed files are backed up first, and running setup again changes nothing. `CORTEX_HOME` moves the whole directory.

### 3. Install the plugin

**Claude Code**

```bash
claude plugin marketplace add <path or git URL of this repository>
claude plugin install cortex@hivellm-cortex
```

The plugin has two optional settings, `bin_dir` (an absolute directory holding `cortex-hook`) and `api_url`. Set them with `/plugin configure cortex@hivellm-cortex`, or leave them empty to use `~/.cortex`.

**Codex**

```bash
codex plugin marketplace add <path or git URL of this repository>
codex plugin add cortex@hivellm-cortex
```

Then open `/hooks` in Codex and trust the Cortex hooks. Codex runs plugin hooks only after you do. Codex hook delivery is not yet verified on Windows; the MCP tools work either way.

### 4. Verify

Start a new Claude Code session. Its first hook starts `cortex-api` and `cortex-adapter-claude`, with logs in `~/.cortex/lifecycle/`. You see a notice only if that start fails. Ask the agent to call `cortex_status`; it should report `runtime_profile: daily` with the API reachable.

### Updating

```bash
cortex update          # dry run: prints the plan
cortex update --apply  # pulls, builds, installs, stops the old runtime
```

`update --apply` runs `git pull --ff-only` in the checkout, builds the five binaries in release mode, and installs them into `~/.cortex/bin` as `cortex install` does. It then stops the `cortex-api` and `cortex-adapter-claude` processes running from that directory, and the next hook starts the new ones. Running `cortex-mcp-server` processes switch on their client's next session. Finally it refreshes the Claude Code and Codex plugins when they are installed; restart Claude Code to load plugin changes. It stops at the first failed step. `--no-pull` builds the checkout as it is. `--source <checkout>` names the checkout when the install manifest does not record it, for example after an install from a relative `--from`. An install also removes from `~/.cortex/bin` any binary the previous install recorded that Cortex no longer installs, such as the `cortex-ops` copy from installs made before these commands moved to `cortex`.

### Upgrading from the absolute-path setup

Earlier installs wrote Cortex hooks and MCP entries into `~/.claude/settings.json`, `~/.claude.json` and `~/.codex/`. After the plugin is installed, remove them:

```bash
python scripts/activate_daily_memory_clients.py --write-manifest m.json            # dry run: review the plan
python scripts/activate_daily_memory_clients.py --apply --expected-manifest m.json
```

Verified backups and a rollback record go to `~/.cortex/migrations/`, and `--rollback <record>` restores the files byte for byte. Until you migrate, the plugin skips events the old hooks already deliver, and `cortex doctor` reports the double install.

### Options and removal

- `CORTEX_HOOK_AUTOSTART=0` keeps starting the runtime manual. Hooks that pass an explicit `--pipe` never start one.
- To remove Cortex, run `claude plugin uninstall cortex@hivellm-cortex` (or `codex plugin remove cortex@hivellm-cortex`) and take `~/.cortex/bin` off PATH. Your data in `~/.cortex` stays.

The [daily runtime guide](docs/operations/daily-memory-runtime.md) covers the runtime in depth, and the [plugin README](packages/cortex-claude-plugin/README.md) covers the plugin contents.

## 🎯 Overview

Daily memory preserves explicitly captured decisions and lessons. The optional full profile adds the broader event and content pipeline:

- **📥 Capture** - Hooks in Claude Code / OpenCode publish every session event (prompts, tool calls, decisions) to a local ingestion bus
- **🏷️ Classify** - Claude Haiku (via Claude Code CLI) enriches events with kind, topics, severity, and PII risk
- **🔎 Index** - Code and docs go to Vectorizer (dense + BM25), events to Meilisearch (full-text), relations to Nexus (graph)
- **🔀 Retrieve** - One hybrid query API (vector + keyword + graph) fused with Reciprocal Rank Fusion and an optional cross-encoder reranker
- **💉 Pre-thinking** - Bounded local memory reaches verified Claude hooks or explicit MCP reads; optional search can add supporting context
- **⚖️ Govern** - Laws with detectors, violations tracking, audit envelopes, and access control (principals + RBAC)
- **📊 Observe** - Desktop dashboard (Electron + React) over the whole substrate

**What Cortex isn't:** a new vector DB, graph DB, search engine, or coding agent. It composes existing HiveLLM services (**Vectorizer**, **Nexus**, **Synap**, **Rulebook**) plus **Meilisearch** and **Claude Haiku** into a capture → classify → retrieve → govern loop. Cortex informs agents; it does not generate code.

## ✨ Key Features

### 📥 Capture & Ingestion
- **Claude Code adapter** - hook shim + long-lived local daemon (`cortex-adapter-claude`), shipped as a [Claude Code plugin](packages/cortex-claude-plugin/README.md) ([spec 10](docs/specs/10-claude-code-adapter.md), [spec 18](docs/specs/18-claude-code-plugin.md))
- **OpenCode adapter** - same envelope contract through the `EnvelopeProducer` trait ([spec 23](docs/specs/23-opencode-adapter.md), [spec 24](docs/specs/24-producer-trait.md))
- **Bootstrap CLI** - one-shot + incremental backfill of existing repos (code, docs, analyses, knowledge, learnings) ([spec 09](docs/specs/09-bootstrap-cli.md))
- **Synap event bus** - durable streams between ingestion and workers, with room-generation-aware self-healing consumers

### 🏷️ Processing Pipeline
- **Classifier worker** - Haiku-backed enrichment with static fallbacks ([spec 05](docs/specs/05-classifier.md))
- **Embedder worker** - Tree-sitter symbol-level code chunks + doc sections, stable content-independent vector ids ([spec 06](docs/specs/06-embedder.md))
- **Graph worker** - sessions, turns, tool calls, artifacts, decisions, laws, and violations projected into Nexus; community detection (Louvain + Leiden) ([spec 07](docs/specs/07-graph-writer.md), [spec 45](docs/specs/45-graph-communities.md))
- **Full-text worker** - per-repo Meilisearch indexes with typo tolerance and faceted filters ([spec 08](docs/specs/08-fulltext-indexer.md))
- **Consolidator** - distills sessions into consolidations that carry context across sessions ([spec 27](docs/specs/27-consolidation.md))

### 🔀 Retrieval
- **Hybrid query API** - `POST /v1/query` fuses vector, keyword, and graph lanes with RRF ([spec 11](docs/specs/11-query-api.md))
- **Reranker** - Text Embeddings Inference cross-encoder on GPU ([spec 37](docs/specs/37-retrieval-rerank.md))
- **Phantom-link verifier** - drops hits that point at symbols/files that do not exist ([spec 28](docs/specs/28-phantom-link-verifier.md))
- **Bitemporal history** - branches, timelines, supersession chains ([spec 30](docs/specs/30-bitemporal-schema.md), [spec 32](docs/specs/32-branches.md), [spec 33](docs/specs/33-timeline-api.md))
- **Eval gate** - golden-set regression suites for retrieval, consolidation, classification, and access control ([spec 29](docs/specs/29-eval.md))

### ⚖️ Governance & Security
- **Laws DSL** - YAML law registry + evaluator; blocking and observational laws ([spec 13](docs/specs/13-laws-dsl.md), [spec 14](docs/specs/14-governance-engine.md))
- **Access control** - principals, RBAC, Bell-LaPadula-style clearance predicates enforced on every lane ([spec 41](docs/specs/41-principal-and-rbac.md), [spec 42](docs/specs/42-access-enforcement.md), [spec 43](docs/specs/43-acl-admin-api.md))
- **Retention + forget** - retention sweeps and an explicit `cortex_forget` path ([spec 19](docs/specs/19-retention.md))
- **Audit envelopes** - every turn's retrieval and decisions are auditable after the fact

### 🤖 Agent Integration
- **MCP server** - 12 stdio tools in daily mode; 42 in the optional full profile ([spec 20](docs/specs/20-mcp-tool-surface.md))
- **Claude Code plugin** - MCP tools + skills + sub-agents + slash commands + capture hooks in one install
- **Runtime discovery** - `cortex_capabilities` lists every registered tool and whether it reads or writes

## 🏗️ Architecture

```
┌───────────────────────────────────────────────────────────────────────┐
│  AI Tools / IDEs                                                      │
│  Claude Code · OpenCode · Cursor · Gemini · Codex · Copilot           │
└───────────────────────────────────────────────────────────────────────┘
                       ▲                       ▲
                       │ capture (hooks)       │ retrieve (MCP / HTTP)
                       ▼                       │
┌───────────────────────────────────────────────────────────────────────┐
│                          C O R T E X                                  │
│  Ingestion → Classify → Embed / Index / Graph → Query API             │
│  + Pre-thinking + Governance + Consolidation + Dashboard              │
└───────────────────────────────────────────────────────────────────────┘
                              ▼
┌───────────────────────────────────────────────────────────────────────┐
│  HiveLLM Data Services                                                │
│  Vectorizer · Nexus · Synap · Meilisearch · TEI reranker · Haiku (CLI)│
└───────────────────────────────────────────────────────────────────────┘
```

Full architecture: [`docs/architecture.md`](docs/architecture.md) · spec index and dependency order: [`docs/specs/00-index.md`](docs/specs/00-index.md).

### Workspace layout

```
crates/
├── cortex-core/                 # Event schema, canonical JSON, ULID, content hash, validator
├── cortex-config/               # Typed config — single source of truth for every CORTEX_* knob
├── cortex-storage/              # Namespaces, schema bootstrap, metadata + CAS stores
├── cortex-workers/              # ingestion, classifier, embedder, fulltext, graph, consolidator, retention, …
├── cortex-api/                  # Hybrid query API + dashboard JSON + retention scheduler
├── cortex-pre-thinking/         # Query bundles → compact Markdown blocks for system prompts
├── cortex-laws/                 # Laws DSL v1 — registry + evaluator
├── cortex-mcp-server/           # stdio MCP bridge to cortex-api / pre-thinking
├── cortex-adapter-claude-code/  # Claude Code hook shim + capture daemon
├── cortex-adapter-opencode/     # OpenCode adapter
├── cortex-cli/                  # cortex (setup/install/update) · cortex-bootstrap · cortex-ops (doctor) · cortex-relevance-eval
├── cortex-eval/                 # Golden-set eval harness (regression gate)
├── cortex-health/               # Shared health-report types + aggregator
└── cortex-build/                # Build-time version stamp (git sha, build ts, dirty flag)
gui/                             # Desktop dashboard — Electron + Vite + React + TypeScript
packages/
├── cortex-claude-plugin/        # Claude Code plugin (MCP, skills, agents, commands, hooks)
└── cortex-opencode-plugin/      # OpenCode plugin
bin/                             # cortex-up / down / reset / logs / doctor (sh + ps1)
docs/                            # architecture, specs, analyses, runbooks
```

## 🚀 Quick Start

For local agent memory, follow [Installation](#-installation). The prerequisites and Compose setup below apply to the optional full stack.

### 📦 Full-stack prerequisites

- **Rust** 1.80+ (stable) — core services and CLIs
- **Docker + Compose** — local data-services stack ([spec 03](docs/specs/03-local-stack.md))
- **Node.js 20+ + pnpm** — dashboard and plugins
- **Claude Code CLI** — default classifier backend
- **NVIDIA GPU** (optional) — for the TEI reranker container

### 🐳 Start the local stack

```bash
cp .env.example .env     # edit secrets before real use
make up                  # Linux/macOS — docker compose up + first-time init
# or
bin/cortex-up.ps1        # Windows
```

This brings up 12 services: Vectorizer, Nexus, Synap, Meilisearch, the TEI reranker, and the Cortex ingestion, classifier, embedder, full-text, graph, API, and Claude-archive services.

```bash
make doctor              # liveness probe against every backend
make logs S=cortex-api   # tail one service
make down                # stop (volumes preserved)
```

### 📚 Index a repo (first time)

Cortex answers `/v1/query`, `cortex_pre_thinking`, and the dashboard only for repos that were bootstrapped at least once. A repo that wasn't returns empty hits with `notice = { code: "repo_not_indexed", … }`, and `cortex_status` lists indexed slugs under `daemon.indexed_repos`.

```bash
cargo run -p cortex-cli --bin cortex-bootstrap -- /path/to/repo
```

After that, calls scoped with `scope.repo = "<slug>"` return real results. The slug is lowercase ASCII — the same form `daemon.indexed_repos` reports.

### 🔌 Connect Claude Code or Codex

Follow [Installation](#-installation). The same plugin install works with the full stack.

### 🖥️ Dashboard

```bash
cd gui
pnpm install
pnpm dev                 # Vite renderer + Electron, talks to cortex-api over HTTP
```

### 🛠️ Build from source

```bash
git clone https://github.com/hivellm/cortex.git
cd cortex

make check               # cargo check --workspace
make clippy              # cargo clippy --workspace --all-targets -- -D warnings
make test                # cargo test --workspace
make build
```

> **Keeping `target/` bounded.** Cargo never garbage-collects build artifacts — `target/` once passed **500 GB** here. The workspace already pins `[profile.dev] debug = "line-tables-only"` and `[profile.release] strip`; on a developer box run `scripts/sweep-target.sh` periodically (needs `cargo install cargo-sweep`) to drop stale artifacts without losing the incremental hot set. Full guide: [`docs/rust-target-hygiene.md`](docs/rust-target-hygiene.md).

### 🌐 Access Points

Default host ports (override via `.env`):

| Service | Host port | Notes |
|---|---|---|
| **cortex-api** | `17000` | `/v1/query`, `/v1/health`, dashboard JSON |
| **Vectorizer** (REST) | `17001` | container `15002` |
| **Vectorizer** (RPC) | `17005` | `vectorizer://`, container `15503` |
| **Nexus** | `17002` | container `15474` |
| **Synap** | `17003` | container `15500` |
| **Meilisearch** | `17004` | container `7700` |
| **cortex-ingestion** | `17010` | envelope intake |
| **cortex-adapter-claude** | `17011` | host-side daemon `/healthz` |
| **classifier / embedder / fulltext / graph** | `17021` – `17024` | per-worker health + admin |
| **cortex-claude-archive** | `17030` | |

## 🔧 MCP Integration

The Cortex plugins carry the MCP entry, so no client or repository file needs
one. The server finds its configuration and credential under `~/.cortex`:

```json
{
  "mcpServers": {
    "cortex": { "command": "cortex-mcp-server", "args": ["serve", "--api-url", "${user_config.api_url}"] }
  }
}
```

An empty `--api-url` falls back to `dashboard.api_url` from the user
configuration. Earlier absolute-path registrations in `~/.claude.json` and
`~/.codex/config.toml` are removed by `scripts/activate_daily_memory_clients.py`.
See the [daily runtime guide](docs/operations/daily-memory-runtime.md).

### Available Tools (41)

**Retrieval (6)**
`cortex_query` · `cortex_pre_thinking` · `cortex_keyword_search` · `cortex_vector_search` · `cortex_topic_search` · `cortex_query_explain`

**Sessions & activity (8)**
`cortex_session_replay` · `cortex_session_timeline` · `cortex_similar_sessions` · `cortex_active_work` · `cortex_events_by_kind` · `cortex_tool_calls` · `cortex_files_touched` · `cortex_timeline`

**Consolidations (7)**
`cortex_consolidation_get` · `cortex_consolidations_recent` · `cortex_consolidations_by_entity` · `cortex_consolidations_search` · `cortex_consolidation_lineage` · `cortex_consolidations_diff` · `cortex_consolidation_costs`

**Graph (4)**
`cortex_graph_query` · `cortex_graph_communities` · `cortex_path` · `cortex_compare`

**Decisions & history (6)**
`cortex_decision_search` · `cortex_decision_chain` · `cortex_history` · `cortex_supersession` · `cortex_branch_list` · `cortex_branch_show`

**Governance & feedback (4)**
`cortex_law_violations` · `cortex_audit` · `cortex_feedback_signals` · `cortex_feedback_record`

**Memory & access control (4)**
`cortex_capture_memory` · `cortex_forget` · `cortex_acl_whoami` · `cortex_acl_grant`

**Health & discovery (2)**
`cortex_status` · `cortex_capabilities`

## 🎯 Use Cases

- **Pre-change context** - before editing a file, pull the decisions, laws, and past sessions that touched it
- **Decision lookup** - "why did we pick X over Y?" with the full supersession chain
- **Cross-session continuity** - a fresh session inherits the distilled context of the previous one
- **Rule compliance** - check an action against active laws; track violations over time
- **Session forensics** - replay what an agent did, which tools it called, and which files it changed

## 🛠️ Technology Stack

- **Core:** Rust (tokio, axum, rusqlite, tree-sitter)
- **Data services:** [Vectorizer](https://github.com/hivellm/vectorizer) (vectors) · [Nexus](https://github.com/hivellm/nexus) (graph) · [Synap](https://github.com/hivellm/synap) (event streams) · [Meilisearch](https://www.meilisearch.com) (full-text)
- **Models:** [Claude Haiku](https://www.anthropic.com/claude/haiku) via Claude Code CLI (classification) · Hugging Face TEI (reranking)
- **Dashboard:** Electron + Vite + React + TypeScript + TanStack Query
- **Governance:** [@hivehub/rulebook](https://github.com/hivellm/rulebook) (tasks, rules, memory)

## 📚 Documentation

- [Architecture](docs/architecture.md) — vision, goals, ecosystem context (read first)
- [Spec index](docs/specs/00-index.md) — 44 specs with per-spec status and dependency order
- [PRD](docs/prd.md) · [Roadmap](docs/roadmap.md) · [Dependency DAG](docs/dag.md)
- [Local stack](docs/specs/03-local-stack.md) — compose topology and pinned service versions
- [MCP tool surface](docs/specs/20-mcp-tool-surface.md)
- [Claude Code plugin](packages/cortex-claude-plugin/README.md)
- [Dashboard](gui/README.md) · [GUI contract](docs/specs/38-gui-contract.md)
- [Operations](docs/operations/) · [Runbooks](docs/runbooks/) · [Observability](docs/observability/) · [Metrics](docs/metrics.md)
- [Analyses](docs/analysis/) — audits, live verifications, platform reviews
- [CHANGELOG](CHANGELOG.md)

## 📍 Status

**0.1.0-alpha, active development.** Spec 01–18 core is implemented and running against the live stack; later phases added consolidation, bitemporal history and branches, access control, retrieval evaluation, reranking, and graph communities. Per-spec status lives in [`docs/specs/00-index.md`](docs/specs/00-index.md); the current roadmap and known issues are in [`docs/analysis/cortex-platform-2026-07/`](docs/analysis/cortex-platform-2026-07/).

## 📄 License

Apache License 2.0 — see [LICENSE](LICENSE).

## 🤝 Contributing

This project is governed by [@hivehub/rulebook](https://github.com/hivellm/rulebook). Before changing anything, read [`AGENTS.md`](AGENTS.md) and [`AGENTS.override.md`](AGENTS.override.md) — the override file holds project laws (strict task order, backlog chaining) that win over generic guidance.

- Track work through the Rulebook MCP (`rulebook_task`); never create task folders by hand
- Work checklists top-to-bottom — first unchecked item in the lowest-numbered section
- Type-check → lint (`clippy -D warnings`) → tests, all green, before every commit
- Conventional commits in English (`feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`)
