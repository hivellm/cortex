# Cortex

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-alpha-yellow.svg)](CHANGELOG.md)

**Local, cited memory for coding agents.**

Cortex preserves decisions, lessons and historical events so agents can recover their sources across sessions. The default product is one Rust executable, `cortex`, with local storage, eleven MCP tools, lifecycle hooks and an embedded browser interface. No daemon, Docker, GPU, Node.js build or external database is required.

Canonical memories are immutable Markdown revisions; raw events live in immutable NDJSON segments. SQLite FTS5, current-record projections and typed relationships are derived indexes that can be rebuilt. Imported events remain observations: importing or classifying them does not promote them into accepted advice.

Start with the [local-first memory guide](docs/operations/local-first-memory.md). The older service-based runtime remains available as an explicitly optional legacy system.

## Install

Build from this checkout with the repository's Rust toolchain:

```bash
cargo build -p cortex-local --bin cortex --release --locked
target/release/cortex install
```

On Windows, the executable is `target/release/cortex.exe`. `install` immediately installs that executable and its embedded plugin bundle; it has no `--apply` or dry-run flag. The default destination is `~/.cortex/bin/cortex` (`cortex.exe` on Windows), with plugins under `~/.cortex/plugins/cortex-standalone`. Existing artifacts are backed up and installed files are checked by SHA-256. Publication of the entire binary/plugin pair is not crash-atomic; preserved backups support recovery.

Use the installed executable by full path, or add `~/.cortex/bin` to PATH for convenient CLI use. Installed MCP manifests already use the absolute executable path and do not require PATH changes. `--home PATH` or `CORTEX_HOME` selects another Cortex home. For a custom home, launch clients with the same `CORTEX_HOME` so lifecycle hooks use it too.

Register the installed local marketplace for each client you use. These examples use the default home in a POSIX shell; in PowerShell, replace `$HOME` with `$env:USERPROFILE`.

```bash
claude plugin marketplace add "$HOME/.cortex/plugins/cortex-standalone"
claude plugin install cortex@hivellm-cortex-local --scope user

codex plugin marketplace add "$HOME/.cortex/plugins/cortex-standalone"
codex plugin add cortex@hivellm-cortex-local
```

Restart the client. In Codex, open `/hooks` and review/trust the five Cortex definitions individually. Changed hook definitions require renewed trust. Windows launchers explicitly invoke CMD so they also work when Codex runs `commandWindows` through PowerShell. Claude Code's shell hooks require its supported shell environment, including Git Bash on Windows.

When migrating, disable the old `cortex@hivellm-cortex` plugin to avoid duplicate delivery; retain historical data and backups. Installing the new executable does not import legacy memories automatically, change native agent memory or register OS services/schedules. See the [migration and recovery guide](docs/operations/local-first-memory.md) before importing existing stores.

## Use

Run inside a repository whose `cortex.toml` declares `[cortex] id`, or pass `--repository` explicitly. Scope is exact and case-sensitive: `Cortex` and `cortex` are different repositories. Directory names do not become repository identities automatically.

```bash
cortex build-info
cortex --repository Cortex status
cortex doctor
cortex --repository Cortex capture --title "Storage decision" --body "Use immutable revisions for accepted decisions." --kind decision --authority accepted
cortex --repository Cortex search "immutable revisions"
cortex --repository Cortex serve
```

`serve` starts only the embedded loopback web interface, on port 17200 by default. Open the token-bearing URL printed by that process. The UI supports search, capture, revisions, historical sources, typed connections and classification queue inspection. No separate frontend build is needed. Treat its session token as a credential; privileged browser extensions can observe browser content and URLs.

CLI commands also cover `get`, `recent`, `revise`, `graph`, historical `import`, `reindex` and explicit `reindex --repair`. Revisions require the expected prior revision. Import preserves original source bytes and repository identities; missing historical scope needs an explicit mapping. Repair retains forensic copies instead of discarding canonical data. Consult command `--help` and the [operations guide](docs/operations/local-first-memory.md) for payloads and recovery steps.

Search uses SQLite FTS5 with literal terms joined by AND. It is not semantic/vector retrieval. Lifecycle recall excludes raw events and inapplicable memories before limiting results, supplements prompt matches with recent applicable memories up to ten candidates, and emits cited context within 6 KiB. Session start uses recent applicable memories directly. Citations include repository, revision, content hash, source and authority. Typed links preserve relationships without silently changing a target's authority.

## Agent integration

Plugins use `cortex mcp` over stdio, without an HTTP service. Inspect descriptors with `cortex mcp --list-tools`.

| Purpose | Implemented tools |
|---|---|
| Search and inspect | `cortex_search`, `cortex_query`, `cortex_get`, `cortex_graph`, `cortex_status` |
| Recall applicable memory | `cortex_pre_thinking`, `cortex_memory_read` |
| Capture and revise | `cortex_capture_memory`, `cortex_revise` |
| Advisory classification queue | `cortex_classification_enqueue`, `cortex_classification_jobs` |

Claude Code and Codex plugins connect five lifecycle events to the same executable. Healthy hooks do not launch services; unavailable, pending or corrupt storage fails open. A delivered context block is not proof that a model used it. OpenCode can use `cortex mcp`; automatic lifecycle capture is not established for its standalone default.

Capture stays local. Optional Jev classification requires explicitly queued candidates and an explicit `cortex classify run` with `TYPESAFE_API_KEY`; only that command contacts TypeSafe. It sends bounded, redacted candidate content to `jev-1.13.0`. Results are advisory, pinned to source revision, and cannot change authority or rewrite the memory. Queue operations in MCP/UI do not themselves call the provider.

## Update

Build the desired source revision, then supply that executable to the installed CLI:

```bash
cargo build -p cortex-local --bin cortex --release --locked
cortex update --binary target/release/cortex
```

On Windows, supply `target/release/cortex.exe`. Update validates the candidate's standalone protocol and installs its own embedded assets together with the binary. It does not pull Git, build source, stop legacy services or refresh client caches automatically. Refresh installed plugin caches with the same client install/add commands, restart clients and renew changed Cortex hook trust, even when the package version is unchanged.

## Verified behavior and limits

The [reproducible evaluation guide](docs/operations/local-memory-evaluation.md) reports development Recall@5 0.8625 and held-out natural-language Recall@5 0.0. A green offline contract/non-regression gate does not establish natural-language readiness or general productivity gains.

[Implementation and installed acceptance evidence](docs/analysis/local-first-core-2026-10/05-implementation-evidence.md) records 128 standalone tests and 4,100 legacy tests passing, with nine pre-existing ignored legacy checks/examples. Check, formatting and strict clippy also passed. Earlier failures and fixture corrections remain recorded rather than replaced by the passing totals.

Fresh Claude and Codex sessions answered two historical questions using the expected memory IDs and revisions with no retrieval tools. Corresponding controls answered "sem fonte". Codex's valid control disabled all hooks for one invocation; a plugin-disable attempt that left hooks active was excluded. This demonstrates observed use of those two memories, not general productivity gains or broad retrieval quality.

Historical acceptance recovered 106,930 unique archive events while preserving source hashes. Sixteen source files remain open compressed frames with no pending line bytes; complete readable lines were recovered, but reports honestly retain `complete=false`. Browser acceptance covered desktop/mobile search, capture, revision, sources and graph navigation using fictional data. One authorized Jev request verified live integration, not classification accuracy.

Remaining limits include literal FTS retrieval, best-effort redaction, explicit client trust/restart, non-atomic binary/plugin pair publication and host-dependent latency. Windows process-crash recovery is tested; equivalent directory-fsync power-loss guarantees are not claimed. See the evidence report for measured bounds and coverage caveats.

## Development

```bash
cargo check -p cortex-local --all-targets --all-features --locked
cargo fmt --all -- --check
cargo clippy -p cortex-local --all-targets --all-features --locked -- -D warnings
cargo test -p cortex-local --all-features --locked
```

`cortex-local` is the default workspace member. The wider workspace retains legacy APIs, adapters, storage and workers; use explicit workspace gates before a push or PR. Follow [AGENTS.md](AGENTS.md), [AGENTS.override.md](AGENTS.override.md) and the task checklist in order. Track multi-session work through Rulebook and run type-check, lint and tests before each commit.

| Path | Role |
|---|---|
| `crates/cortex-local/` | Standalone executable, canonical store, FTS/links, import, MCP, hooks and embedded web UI |
| `packages/cortex-claude-plugin/` | Claude Code plugin assets |
| `packages/cortex-codex-plugin/` | Codex plugin assets |
| `crates/cortex-cli/` | Legacy operator executable `cortex-legacy` and legacy utilities |
| `crates/cortex-api/`, `crates/cortex-workers/` | Optional legacy service pipeline |
| `gui/` | Legacy Electron dashboard |

Cargo does not automatically reclaim old build artifacts. See [Rust target hygiene](docs/rust-target-hygiene.md) for bounded local maintenance.

## Legacy stack and documentation

Vectorizer, Nexus, Synap, Meilisearch, model workers, GPU reranking and the Electron dashboard remain optional **legacy** capabilities. Their daemon, bootstrap, Compose, API and multi-binary instructions do not apply to the standalone quick start. The retained legacy operator is `cortex-legacy`, not `cortex`; older documents may use its former name.

- [Local-first operations and migration](docs/operations/local-first-memory.md)
- [Installed acceptance and measured limits](docs/analysis/local-first-core-2026-10/05-implementation-evidence.md)
- [Legacy daily runtime guide](docs/operations/daily-memory-runtime.md)
- [Legacy architecture](docs/architecture.md) and [spec index](docs/specs/00-index.md)
- [Legacy local stack](docs/specs/03-local-stack.md) and [MCP surface](docs/specs/20-mcp-tool-surface.md)
- [Legacy dashboard](gui/README.md)
- [Operations](docs/operations/), [runbooks](docs/runbooks/) and [CHANGELOG](CHANGELOG.md)

## License

Apache License 2.0. See [LICENSE](LICENSE).
