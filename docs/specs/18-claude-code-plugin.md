# 18 — Claude Code plugin (skills + agents + commands + MCP server)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 10, 11, 12

## Goal

Ship Cortex as a first-class **Claude Code plugin** so the assistant can call into Cortex from inside a session — querying for decisions, surfacing active laws, refreshing pre-thinking context, auditing past turns. Spec 10 covers capture (hooks shovelling events into Cortex). This spec covers the inverse direction: surfaces the model itself reaches for.

A Claude Code plugin is a **directory of text + config**, not a Rust crate. The Cortex plugin ships:

- A `.claude-plugin/plugin.json` manifest.
- An `.mcp.json` that registers our **MCP server binary** (Rust, lives in this repo) so the model can call `cortex_query`, `cortex_pre_thinking`, `cortex_status` mid-turn. Tool names are identifier-safe per MCP 2024-11-05 (no `.`); descriptors use `inputSchema` (camelCase). Hosts silently drop tools that violate either rule.
- Markdown **skills** (`skills/cortex-*/SKILL.md`) the model can invoke.
- Markdown **sub-agents** (`agents/cortex-*.md`) for focused tasks.
- Markdown **slash commands** (`commands/cortex-*.md`) the human runs verbatim.
- A `.claude-plugin/marketplace.json` so users install via `/plugin marketplace add hivellm/cortex` → `/plugin install cortex@hivellm`.

## Scope

**In:**
- `packages/cortex-claude-plugin/` directory at the workspace root with the canonical Claude Code plugin layout.
- `cortex-mcp-server` Rust crate — language-agnostic MCP server binary referenced from `packages/cortex-claude-plugin/.mcp.json`. JSON-RPC 2.0 over stdio (canonical MCP transport).
- Three MCP tools backed by existing services: `cortex_query` (spec 11), `cortex_pre_thinking` (spec 12), `cortex_status` (Cortex daemon health).
- Three skills authored as Markdown: `cortex-context`, `cortex-audit`, `cortex-laws`.
- Three sub-agents authored as Markdown: `cortex-historian`, `cortex-lawkeeper`, `cortex-context-curator`.
- Six slash commands authored as Markdown: `cortex-status`, `cortex-query`, `cortex-laws`, `cortex-decisions`, `cortex-pre-thinking`, `cortex-audit`.
- Distribution via a Cortex marketplace JSON pointing back at this Git repo.

**Out:**
- VS Code marketplace extension — separate Phase-3 effort.
- Non-Claude-Code MCP hosts (Cursor / Codex / Gemini) — spec 17 covers those.
- Re-implementing the query API or pre-thinking pipeline — the plugin is a thin adapter.

**Hook registration (capture, spec 10) was originally out-of-scope but is folded in via the plugin's `hooks/hooks.json` so a single `claude plugin install cortex@hivellm-cortex` wires up both directions: the model can pull from Cortex via MCP tools, and the Claude Code host pushes session events into Cortex via the plugin-installed hooks. The standalone `cortex-adapter-claude install` path is retained for non-plugin users; pass `--no-hooks` when both paths are present to avoid duplicate firing.**

## Inputs / Outputs

### Plugin directory layout

```
packages/cortex-claude-plugin/                          # workspace-root plugin directory
├── .claude-plugin/
│   ├── plugin.json                    # required manifest (per Claude Code docs)
│   └── marketplace.json               # marketplace listing (used when this repo is added as a marketplace)
├── README.md                          # user docs surfaced by `/plugin info`
├── .mcp.json                          # MCP server registrations
├── skills/
│   ├── cortex-context/SKILL.md
│   ├── cortex-audit/SKILL.md
│   └── cortex-laws/SKILL.md
├── agents/
│   ├── cortex-historian.md
│   ├── cortex-lawkeeper.md
│   └── cortex-context-curator.md
├── commands/
│   ├── cortex-status.md
│   ├── cortex-query.md
│   ├── cortex-laws.md
│   ├── cortex-decisions.md
│   ├── cortex-pre-thinking.md
│   └── cortex-audit.md
└── hooks/                              # spec-10 capture surface, registered at install
    ├── hooks.json                      # event → bash "${CLAUDE_PLUGIN_ROOT}/hooks/cortex-*.sh"
    ├── cortex-session-start.{sh,ps1}
    ├── cortex-user-prompt.{sh,ps1}
    ├── cortex-pre-tool.{sh,ps1}
    ├── cortex-post-tool.{sh,ps1}
    ├── cortex-stop.{sh,ps1}
    ├── cortex-subagent-stop.{sh,ps1}
    └── cortex-notification.{sh,ps1}
```

Per the Claude Code plugin reference, every component is auto-discovered from this layout. A single `plugin.json` is the only required manifest; subdirectories carry their own component-level metadata (YAML frontmatter for agents, `.mcp.json` for MCP, etc).

### `plugin.json` manifest

```jsonc
{
  "name": "cortex",
  "description": "Cortex retrieval, pre-thinking context, law check, and audit surfaces for Claude Code sessions.",
  "version": "0.1.0",
  "author": { "name": "HiveLLM", "email": "hello@hivellm.org" },
  "homepage": "https://github.com/hivellm/cortex",
  "repository": { "type": "git", "url": "https://github.com/hivellm/cortex" },
  "license": "Apache-2.0"
}
```

### `.mcp.json` (MCP server registration)

```jsonc
{
  "mcpServers": {
    "cortex": {
      "command": "cortex-mcp-server",
      "args": ["serve"],
      "env": {
        "CORTEX_API_URL": "http://127.0.0.1:17000",
        "CORTEX_ADAPTER_SOCK": "~/.cortex/adapter-claude.sock"
      }
    }
  }
}
```

The `cortex-mcp-server` binary ships from this repo (Rust crate `cortex-mcp-server`). Once the user runs `cargo install --path crates/cortex-mcp-server` (or grabs a release artifact), the binary is on `PATH` and Claude Code spawns it on plugin activation.

### MCP wire protocol

JSON-RPC 2.0 over stdio (Anthropic's MCP spec, revision `2024-11-05`). Standard methods:

```jsonc
// initialize handshake
→ { "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {...} } }
← { "jsonrpc": "2.0", "id": 1, "result": {
      "protocolVersion": "2024-11-05",
      "capabilities": { "tools": {} },
      "serverInfo": { "name": "cortex-mcp-server", "version": "0.1.0" }
    } }

// notifications/initialized (notification, no id)
→ { "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} }

// tools/list
→ { "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }
← { "jsonrpc": "2.0", "id": 2, "result": { "tools": [ <descriptor>, ... ] } }

// tools/call
→ { "jsonrpc": "2.0", "id": 3, "method": "tools/call",
    "params": { "name": "cortex_query", "arguments": { "intent": "free_search", "query": "ef_search" } } }
← { "jsonrpc": "2.0", "id": 3, "result": {
      "content": [ { "type": "text", "text": "<JSON or formatted bundle>" } ],
      "isError": false
    } }
```

Errors use the standard JSON-RPC codes (`-32700` parse, `-32600` invalid request, `-32601` method not found, `-32602` invalid params, `-32603` internal) plus a `data` field carrying the spec-11 reason (`empty_query`, `scope_forbidden`, `rate_limited`, `scope_repo_required`).

### Header injection contract (phase6a)

The MCP server attaches identifying headers to every outbound `/v1/query` call so the daemon's [scope-resolution lanes](11-query-api.md#scope-resolution-phase6a) can derive `scope.repo` for callers that did not provide it explicitly:

| Header           | Source                                                                                  | Daemon lane (audit value) |
|------------------|-----------------------------------------------------------------------------------------|---------------------------|
| `x-cortex-caller`| Static `claude-code-plugin` identifier — used for ACL + rate limiting                   | n/a                       |
| `x-cortex-cwd`   | `std::env::current_dir()` of the MCP server process (which inherits the operator's `cwd`) | `cwd` (basename → slug)   |
| `x-cortex-repo`  | Reserved for callers that already know the canonical slug (dashboard sidebar today)     | `header`                  |

When the tool's `arguments.scope.repo` is set, the daemon resolves through the `explicit` lane and the headers are informational. When it is missing, `x-cortex-cwd` carries the fallback signal — a `cortex_query` call from `e:/HiveLLM/Cortex` resolves to `scope.repo = "cortex"` without any model-side scope plumbing.

If every lane misses, the daemon returns `422 { "reason": "scope_repo_required" }`. The MCP tool surfaces this as a JSON-RPC `result` with `isError = true` and `data.reason = "scope_repo_required"` so the host can render an actionable hint instead of the silent zero-hit response that motivated the lane (F-003 in the relevance audit). The legacy `cortex-unknown-{family}` fallback is gated behind `CORTEX_ALLOW_UNKNOWN_SCOPE=1` for one deprecation window and removed at the harness gate (`phase6e`).

### Tool descriptors

`cortex_query` reuses `cortex_api::tool_descriptor()` byte-for-byte (spec 11 §MCP tool binding) so the schema source-of-truth stays in `cortex-api`.

`cortex_pre_thinking`:
```jsonc
{
  "name": "cortex_pre_thinking",
  "description": "Run the spec-12 pre-thinking pipeline against the configured cortex-api and return the deterministic Markdown bundle.",
  "inputSchema": {
    "type": "object",
    "required": ["user_prompt", "cwd"],
    "properties": {
      "user_prompt": { "type": "string" },
      "cwd": { "type": "string" },
      "session_id": { "type": "string" },
      "turn_id": { "type": "string" },
      "budget_bytes": { "type": "integer", "default": 32768 },
      "budget_ms": { "type": "integer", "default": 600 }
    }
  }
}
```

`cortex_status`:
```jsonc
{
  "name": "cortex_status",
  "description": "Cortex daemon health snapshot: pid, queue depth, recent publisher errors, overflow WAL bytes.",
  "inputSchema": { "type": "object", "properties": {} }
}
```

The wrapped `daemon` payload echoes the upstream `/v1/status` shape verbatim:

```jsonc
{
  "service": "cortex-api",
  "version": "0.1.0",
  "pid": 12345,
  "uptime_ms": 91234,
  // Sorted slugs the daemon currently has signal for. Callers use it
  // to detect "this repo was never indexed" before running a query —
  // pairs with `notice.repo_not_indexed` on `/v1/query` (issue #1).
  "indexed_repos": ["cortex", "vectorizer"]
}
```

#### Contract guardrails

Tool descriptors MUST satisfy MCP 2024-11-05:

- **Names** match `[a-zA-Z0-9_-]+`. `.` and other punctuation are forbidden — Claude Code drops the descriptor silently when violated. Confirmed 2026-04-27 by `phase2_mcp_server_contract_fix`: the dotted names `cortex.query` / `cortex.pre_thinking` / `cortex.status` shipped with the 0.1.0 plugin and were never callable from the model despite the MCP panel showing `Connected`.
- **Schema fields** use camelCase `inputSchema` / `outputSchema`. Snake_case `input_schema` / `output_schema` is rejected by the same loader for the same reason. Tests in `cortex-mcp-server::tools` and `cortex-api::mcp` lock the camelCase shape and assert the snake_case keys are absent.

### Skills (`skills/cortex-*/SKILL.md`)

Each skill is a Markdown file with YAML frontmatter declaring the skill's name, description, and tool requirements. The model invokes them when the user describes a matching intent:

- `cortex-context` — refresh the system-prompt context bundle by calling `cortex.pre_thinking`.
- `cortex-audit` — fetch the audit envelope for a turn id.
- `cortex-laws` — list active laws in scope via `cortex.query` with `intent=law_check`.

### Sub-agents (`agents/cortex-*.md`)

Each sub-agent is a Markdown file with YAML frontmatter (per Claude Code agent reference) declaring the model + system prompt + allowed tools:

- `cortex-historian` — specialised at `decision_lookup`; emphasises supersession chains + dates.
- `cortex-lawkeeper` — runs `intent=law_check` + reasons about whether the proposed action triggers any active law.
- `cortex-context-curator` — picks the right intent + scope + returns a focused bundle.

### Slash commands (`commands/cortex-*.md`)

User-invoked. Each Markdown file is a prompt template the slash command expands to.

### Hooks (`hooks/`)

The plugin ships the spec-10 hook shims directly so capture activates the moment the plugin is installed. `hooks/hooks.json` follows the canonical Claude Code plugin shape: each Claude Code event (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, `Notification`) maps to one matcher group invoking `bash "${CLAUDE_PLUGIN_ROOT}/hooks/cortex-<event>.sh"` with a 5-second timeout. The shim scripts under `hooks/` are byte-identical mirrors of `crates/cortex-adapter-claude-code/hooks/`; a CI drift test (`cargo test -p cortex-mcp-server --test hook_drift`) refuses to land a change to one tree without the matching change in the other. The shims forward each event to `~/.cortex/adapter-claude.sock`, which the spec-10 daemon publishes into Cortex via the existing publisher / WAL pipeline.

### Marketplace JSON

```jsonc
{
  "name": "hivellm-cortex",
  "owner": { "name": "HiveLLM" },
  "plugins": [
    {
      "name": "cortex",
      "description": "...",
      "version": "0.1.0",
      "source": {
        "type": "git",
        "url": "https://github.com/hivellm/cortex",
        "path": "packages/cortex-claude-plugin"
      }
    }
  ]
}
```

Users add the marketplace once (`/plugin marketplace add hivellm/cortex`) and install via `/plugin install cortex@hivellm-cortex`.

## Design

### Plugin directory at workspace root

The plugin directory lives at `packages/cortex-claude-plugin/` — separate from `crates/` since it's not a Rust crate. Authored as text + JSON, no build step. CI lints it via a small Rust binary (`cortex-mcp-server` ships a `validate` sub-command that checks every asset against the Claude Code reference schema).

### `cortex-mcp-server` Rust crate

Location: `crates/cortex-mcp-server/`. Single binary, three tools, stdio transport. The binary is **the only Rust artifact** in this spec — everything else is text the plugin directory carries. The binary is referenced by the plugin's `.mcp.json` and must be on `PATH`; users install it via `cargo install --path` or a release artifact.

```
crates/cortex-mcp-server/
├── Cargo.toml
├── src/
│   ├── main.rs               (binary entry — clap sub-commands: serve, validate, print-tool-descriptors)
│   ├── lib.rs
│   ├── rpc.rs                (JSON-RPC 2.0 framing + Request / Response / Error)
│   ├── server.rs             (handshake state, dispatch loop)
│   ├── tools.rs              (Tool trait + registry + 3 concrete tools)
│   ├── transport_stdio.rs    (newline-delimited JSON over stdin/stdout)
│   ├── validate.rs           (lint packages/cortex-claude-plugin/ against the Claude Code reference)
│   └── metrics.rs            (cortex.plugin.* counters)
└── tests/
```

### Tool implementations

- `QueryTool` — wraps `cortex_api::QueryService` via HTTP POST to `<api_url>/v1/query`.
- `PreThinkingTool` — calls `cortex_pre_thinking::pipeline::run` with a `QueryFn` that POSTs to `<api_url>/v1/query`.
- `StatusTool` — reads `<api_url>/v1/status` (or the adapter's overflow WAL gauge file directly when the API is unreachable).

### Phase19 — granular tool surface (registry 13 → 29)

The MCP registry ships 29 tools after phase19. The 13 baseline
tools (`cortex_query`, `cortex_pre_thinking`, `cortex_status`,
`cortex_audit`, `cortex_capture_memory`, `cortex_session_replay`,
`cortex_forget`, `cortex_keyword_search`, `cortex_vector_search`,
`cortex_graph_query`, `cortex_active_work`,
`cortex_similar_sessions`, `cortex_decision_chain`) keep their
spec-11 / spec-12 / spec-22 contracts unchanged. Phase19 adds
the 16 granular verbs below — every entry points at the matching
endpoint on `cortex-api` (full wire shape + scope deviations
live in spec 22 "Phase19 — Granular tool surface").

#### Group A — envelope-shape granularity

- `cortex_events_by_kind` → `POST /v1/search/events`
- `cortex_session_timeline` → `GET /v1/sessions/{session_id}/timeline`
- `cortex_tool_calls` → `POST /v1/search/tool-calls`
- `cortex_files_touched` → `GET /v1/sessions/{session_id}/files-touched` OR `POST /v1/search/files-touched`
- `cortex_topic_search` → `POST /v1/topic-cards/search`

#### Group B — consolidation-first

- `cortex_consolidation_get` → `GET /v1/consolidations/{id}`
- `cortex_consolidations_recent` → `GET /v1/consolidations/recent`
- `cortex_consolidations_by_entity` → `POST /v1/consolidations/by-entity`
- `cortex_consolidations_search` → `POST /v1/consolidations/search`
- `cortex_consolidation_lineage` → `GET /v1/consolidations/{id}/lineage`
- `cortex_consolidations_diff` → `GET /v1/consolidations/diff`

#### Group C — governance + telemetry

- `cortex_law_violations` → `POST /v1/laws/violations`
- `cortex_feedback_signals` → `POST /v1/feedback/list`
- `cortex_decision_search` → `POST /v1/decisions/search`
- `cortex_consolidation_costs` → `POST /v1/consolidations/costs`
- `cortex_query_explain` → `POST /v1/query/explain`

Each tool is implemented by a struct in
`crates/cortex-mcp-server/src/tools.rs`, re-exported on
`cortex_mcp_server::lib`, and registered in
`ToolRegistry::default_set()`. The `tools/list` round-trip
asserts `arr.len() == 29`
(`crates/cortex-mcp-server/src/server.rs::tests::tools_list_returns_twentynine_descriptors`).

### Failure modes

| Failure                                        | Handling                                                                |
|------------------------------------------------|-------------------------------------------------------------------------|
| `cortex-api` unreachable                       | Tool returns `result.isError = true` with `reason=api_unreachable`      |
| `cortex-api` 4xx                                | Mirrored as JSON-RPC error with the spec-11 reason in `data`            |
| Malformed JSON-RPC frame                        | Server emits `-32700 parse error` and stays alive                       |
| Unknown tool name                               | Server emits `-32601 method not found`                                  |
| Tool panic                                      | Caught by the dispatch wrapper; returns `-32603 internal error`          |
| Plugin asset corruption                          | `cortex-mcp-server validate` fails CI; the plugin doesn't ship           |

### Observability

```
cortex.plugin.tool.invocations         counter, labels: tool
cortex.plugin.tool.latency_ms          histogram, labels: tool
cortex.plugin.tool.errors              counter, labels: tool, reason
cortex.plugin.session.handshakes       counter
```

Every tool invocation also emits a structured tracing event with `tool`, `latency_ms`, `outcome`.

## Acceptance criteria

- [ ] `packages/cortex-claude-plugin/.claude-plugin/plugin.json` parses as a valid Claude Code plugin manifest (`name`, `description`, `version`, `author`).
- [ ] `packages/cortex-claude-plugin/.mcp.json` references `cortex-mcp-server` with the documented env vars.
- [ ] `packages/cortex-claude-plugin/skills/`, `packages/cortex-claude-plugin/agents/`, `packages/cortex-claude-plugin/commands/` each contain the documented files; YAML frontmatter parses on every agent.
- [ ] `cortex-mcp-server validate ./packages/cortex-claude-plugin` exits 0 on a clean tree and non-zero with a clear message when a required file is missing or malformed.
- [ ] Spawning `cortex-mcp-server serve` and sending an `initialize` over stdio returns a valid response with `tools` capability advertised.
- [ ] `tools/list` returns 29 descriptors (13 baseline + 16 phase19 granular verbs); `cortex_query`'s descriptor matches `cortex_api::tool_descriptor()` byte-for-byte.
- [ ] `tools/call` for `cortex.query` against a wiremock'd `cortex-api` returns the spec-11 response shape.
- [ ] `tools/call` for `cortex.pre_thinking` returns a Markdown bundle with the spec-12 `<!-- cortex: ... query_id=... -->` leading comment.
- [ ] `tools/call` for `cortex.status` returns daemon pid + queue depth + WAL bytes.
- [ ] Unknown tool name → JSON-RPC `-32601 method not found`.
- [ ] Malformed JSON frame → `-32700 parse error` and the server stays alive for the next message.
- [ ] Local install drill: `claude --plugin-dir ./packages/cortex-claude-plugin` boots, `/plugin list` shows `cortex`, `tools/list` over the embedded MCP server returns the three tools.
- [ ] Marketplace listing: `packages/cortex-claude-plugin/.claude-plugin/marketplace.json` parses and points at this repo's `packages/cortex-claude-plugin/` path.
- [ ] Telemetry counters non-zero after a 5-tool-call recorded session.
- [ ] `packages/cortex-claude-plugin/hooks/hooks.json` parses; every script it references exists; every shim under `hooks/` is referenced from `hooks.json`; `cortex-mcp-server validate` enforces both invariants.
- [ ] Hook shims under `packages/cortex-claude-plugin/hooks/cortex-*.{sh,ps1}` are byte-identical to the canonical sources under `crates/cortex-adapter-claude-code/hooks/` (drift test fails the build otherwise).
- [ ] After `claude plugin install cortex@hivellm-cortex`, a fresh Claude Code session emits `UserPromptSubmit` events that reach `~/.cortex/adapter-claude.sock` and the spec-10 daemon publishes them upstream (capture round-trip).
- [ ] `cortex-adapter-claude install --no-hooks` keeps `~/.claude/settings.json` byte-identical to its pre-install state and writes zero shims, so spec-18 plugin users can run the standalone install side-by-side without duplicate hook firing.

## Decisions

1. **Plugin = text directory, server = Rust binary.** Plugins are not language-coupled; the only Rust artifact is the MCP server binary referenced from `.mcp.json`. Skills / agents / commands are authored as Markdown the model interprets directly.
2. **Stdio is canonical.** SSE is reserved for future browser-side scenarios; v1 ships stdio only.
3. **Tool descriptors live in `cortex-api`.** `cortex.query` reuses `cortex_api::tool_descriptor()` so the schema source-of-truth stays in one crate.
4. **Plugin lives at workspace root, not under `crates/`.** It's an artifact, not a Rust package.
5. **CI validates the asset tree.** `cortex-mcp-server validate ./packages/cortex-claude-plugin` runs in CI; the plugin can't ship with a missing file or a malformed agent frontmatter.
6. **Distribution via a Cortex marketplace.** `packages/cortex-claude-plugin/.claude-plugin/marketplace.json` lets users `/plugin marketplace add hivellm/cortex` and pull updates from this repo.
7. **JSON-RPC errors carry the spec-11 reason in `data`.** Hosts that surface error messages to the user get a deterministic string they can pattern-match on.
8. **Hooks ship inside the plugin tree.** The `hooks/` directory and `hooks/hooks.json` register capture at plugin-install time so a single `claude plugin install` covers both pull (MCP tools) and push (capture). The spec-10 standalone install path stays for non-plugin users; the new `--no-hooks` flag keeps both paths cohabitable.
9. **Single source of truth for hook shims.** Canonical `cortex-*.{sh,ps1}` live under `crates/cortex-adapter-claude-code/hooks/`. The plugin tree mirrors them and a drift test refuses divergence — no need to re-author Bash scripts inside the plugin tree.

## Timeout contract (phase14i)

Every MCP tool call is bounded by a per-tool timeout enforced by
the `cortex-mcp-server` dispatcher BEFORE the call reaches the
tool body. Default is `MCP_TOOL_TIMEOUT_MS = 5_000` (5 s); per-tool
overrides live on `ToolContext::tool_timeouts` and are populated
from the env knobs `CORTEX_MCP_TOOL_TIMEOUT_MS` (default) and
`CORTEX_MCP_TOOL_TIMEOUT_MS_{TOOL_NAME}` (per-tool override) when
wired through the future `cortex_config::McpConfig`.

Wire shape when a tool exceeds its budget:

```json
{
  "jsonrpc": "2.0",
  "id": <request_id>,
  "error": {
    "code": -32603,
    "message": "tool `<name>` exceeded timeout after <elapsed_ms> ms",
    "data": {
      "reason": "tool_timeout",
      "elapsed_ms": <u64>,
      "tool": "<mcp tool name>",
      "request_id": <jsonrpc id>
    }
  }
}
```

Callers pattern-match on `error.data.reason == "tool_timeout"`.
The structured error replaces the legacy silent fall-through to a
generic empty result (which was indistinguishable from "tool
returned empty"). `cortex_mcp_server::tools::reasons::TOOL_TIMEOUT`
is the canonical constant.

`ToolError::timeout(tool, elapsed_ms)` is the in-crate constructor
that surfaces this shape; tools that want to short-circuit early
(e.g. with their own internal `tokio::time::timeout` around a
slow sub-call) MAY return it directly. The dispatcher uses
`tokio::time::timeout` around the join handle so a tool whose
body never returns (deadlocked async loop, blocking syscall) is
still cancelled and reported.

## Open questions

1. **MCP stateful resources.** Should `cortex.session` expose the active turn / decisions / laws as MCP resources (URIs the host can subscribe to)? Defer to Phase 3.

## References

- Claude Code plugin reference: https://code.claude.com/docs/en/plugins.md
- Claude Code plugin reference (full schema): https://code.claude.com/docs/en/plugins-reference.md
- Claude Code marketplaces: https://code.claude.com/docs/en/plugin-marketplaces.md
- Claude Code MCP integration: https://code.claude.com/docs/en/mcp.md
- Anthropic MCP spec: https://modelcontextprotocol.io
- JSON-RPC 2.0: https://www.jsonrpc.org/specification
- Spec 10 — Claude Code adapter (hook-based capture).
- Spec 11 — Query API (tool descriptor source).
- Spec 12 — Pre-thinking injection (the `cortex.pre_thinking` tool wrap).
- Spec 17 — Additional adapters (this plugin is the reference for non-hook integration).
