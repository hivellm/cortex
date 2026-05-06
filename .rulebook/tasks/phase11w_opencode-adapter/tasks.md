## 1. Phase 0 — Spike & open questions
- [ ] 1.1 Install OpenCode locally (CLI + Bun runtime). Document version pinned in `docs/analysis/opencode-adapter/00-spike.md`.
- [ ] 1.2 Create a throwaway project; register `cortex-mcp-server` (stdio) under `opencode.json` `mcp.cortex.type=local`. Confirm `cortex_query`, `cortex_status`, `cortex_pre_thinking` appear in the OpenCode MCP tool listing.
- [ ] 1.3 Write a 50-line probe plugin in `.opencode/plugins/probe.ts` that subscribes to `session.created`, `message.updated`, `tool.execute.before`, `tool.execute.after`, `tui.prompt.append`, `permission.asked`, `session.idle`. Log every event payload to `/tmp/opencode-probe.jsonl`.
- [ ] 1.4 Run a representative session (1 prompt + 2 tool calls + 1 subagent invocation). Capture the JSONL.
- [ ] 1.5 Answer in `docs/analysis/opencode-adapter/00-spike.md`:
  (a) which event(s) fire on user prompt submission and in what order;
  (b) whether `tool.execute.before` returning a Promise blocks the tool call until resolved;
  (c) whether `tui.prompt.append` mutates the **current** model call or buffers next-turn;
  (d) whether `permission.asked` reply from a plugin can deny the tool;
  (e) whether `session.idle` fires per-subagent or only on outer-turn end.
- [ ] 1.6 Decide: pre-thinking injection path (A: `tui.prompt.append`, B: SDK priming, C: `/cortex-prime` command). Record decision + reasoning in the spike doc.

## 2. Schema + adapter daemon HTTP listener
- [ ] 2.1 Add `"opencode"` to the `tool` enum in `crates/cortex-core/schemas/envelope.schema.json`. Regenerate any derived TS types.
- [ ] 2.2 Add `pub const TOOL_OPENCODE: &str = "opencode";` next to `TOOL_CLAUDE_CODE` in `crates/cortex-adapter-claude-code/src/events.rs`. Re-export from `lib.rs`.
- [ ] 2.3 In `crates/cortex-adapter-claude-code/src/ipc.rs`, refactor `IpcBinding` so it accepts multiple transports. Today: Unix socket (Linux/macOS), named pipe (Windows). Add: HTTP listener.
- [ ] 2.4 New `HttpBinding` variant: parses `CORTEX_ADAPTER_HTTP_BIND` env (default `127.0.0.1:17004`); uses `axum` 0.7 (already a workspace dep) to serve `POST /hook` accepting the same JSON payload as the socket path.
- [ ] 2.5 Both bindings funnel into the same `Dispatcher::dispatch` entrypoint. No envelope-shape changes.
- [ ] 2.6 Concurrent-binding test: start daemon with both transports; post via both; assert both produce identical envelopes (`crates/cortex-adapter-claude-code/tests/dispatcher.rs::http_and_socket_parity`).
- [ ] 2.7 Document the new env var in `crates/cortex-adapter-claude-code/README.md` § Configuration.

## 3. Spec — `docs/specs/20-opencode-adapter.md`
- [ ] 3.1 Author the new spec mirroring spec 10's structure: §Hook contract, §Envelope mapping, §Sync paths, §Configuration, §Plugin contract, §Stability.
- [ ] 3.2 Spec 10 §Envelope mapping table copied with a column added for the OpenCode event name.
- [ ] 3.3 Cross-reference: spec 11 (`/v1/query`), spec 12 (pre-thinking), spec 18 (MCP server) — unchanged for OpenCode.
- [ ] 3.4 Document the four resolved spike answers as load-bearing constraints.

## 4. TS plugin package — `packages/cortex-opencode-plugin/`
- [ ] 4.1 Create the package skeleton: `package.json` (name `@hivellm/cortex-opencode-plugin`, peer dep `@opencode-ai/plugin@^X` pinned to spike-verified version), `tsconfig.json`, `src/`, `test/`.
- [ ] 4.2 `src/events.ts`: enum + mapper from OpenCode event names → adapter `HookKind` strings (must match the Rust enum names verbatim to avoid daemon-side branching).
- [ ] 4.3 `src/client.ts`: `postHook(kind, event, ctx)` → `fetch(CORTEX_ADAPTER_HTTP_BIND + '/hook', {method: 'POST', body: …})` with 1500 ms timeout. Fail-open on network errors. Reuse the `additionalContext` envelope shape the daemon already returns.
- [ ] 4.4 `src/scope.ts`: derive repo slug from `directory` + `worktree` using the same heuristics as `cortex-adapter-claude-code/src/dispatcher.rs::scope`. Cache per session_id.
- [ ] 4.5 `src/config.ts`: load env vars (`CORTEX_ADAPTER_HTTP_BIND`, `CORTEX_OPENCODE_DISABLE`, `CORTEX_OPENCODE_PRE_THINKING_KB`, `CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS`).
- [ ] 4.6 `src/index.ts`: export `CortexPlugin` async factory. Subscribe to: `session.created` → SessionStart; user-prompt event from §1.5(a) → UserPromptSubmit; `tool.execute.before` → PreToolUse; `tool.execute.after` → PostToolUse; `session.idle` → Stop; `permission.asked` → optional law-check.
- [ ] 4.7 Pre-thinking injection per §1.6 decision (Path A `tui.prompt.append`, B SDK priming, or C `/cortex-prime` command).
- [ ] 4.8 Unit tests in `test/events.test.ts`: kind mapping, scope derivation, fail-open on adapter unreachable.
- [ ] 4.9 Build script (`tsc` → `dist/`) + `prepublishOnly`.

## 5. Project config — `opencode.json`
- [ ] 5.1 Author `opencode.json` at repo root (NOT in user config) with:
  - `mcp.cortex.type=local`, command + env (port from `.mcp.json`).
  - `mcp.rulebook.type=local`, command `["npx", "-y", "@hivehub/rulebook@latest", "mcp-server"]`.
  - `plugin: ["@hivellm/cortex-opencode-plugin"]` (or local path during dev).
  - `agent: { ... }` keys for any agents that prefer JSON over markdown.
  - `instructions` pointing to `AGENTS.md` + `AGENTS.override.md` if OpenCode's `instructions` key supports a list.
- [ ] 5.2 Verify the config validates against OpenCode's published JSON schema (if one exists; otherwise lint via `opencode --validate-config` if available).

## 6. Custom commands port — `.opencode/commands/`
- [ ] 6.1 List every file in `.claude/commands/` (14 today: `rulebook-decision-create`, `rulebook-decision-list`, `rulebook-knowledge-add`, `rulebook-knowledge-list`, `rulebook-learn-capture`, `rulebook-learn-list`, `rulebook-memory-save`, `rulebook-memory-search`, `rulebook-task-apply`, `rulebook-task-archive`, `rulebook-task-create`, `rulebook-task-list`, `rulebook-task-show`, `rulebook-task-validate`).
- [ ] 6.2 For each, write `.opencode/commands/<same-name>.md` with frontmatter `template:` carrying the existing prompt body. Preserve `$ARGUMENTS` placeholders.
- [ ] 6.3 Spot-check three commands inside an OpenCode session: invoke `/rulebook-task-list`, `/rulebook-decision-list`, `/rulebook-memory-search foo`. Confirm output matches Claude Code parity.

## 7. Agents port — `.opencode/agents/`
- [ ] 7.1 Enumerate `.claude/agents/*.md` (project + user + plugin-provided). For each, decide primary vs subagent (most are subagents).
- [ ] 7.2 Translate each to `.opencode/agents/<name>.md`:
  - Frontmatter: `description`, `model`, `temperature`, `max_steps`, `permission` block (`bash`, `edit`, `read`, `write` → `allow`/`ask`/`deny`, with glob patterns from the Claude Code `allowed-tools` list).
  - Body: existing prompt verbatim or via `prompt: "{file:./prompt.md}"` if longer.
- [ ] 7.3 Spot-check three agents: invoke `@researcher`, `@implementer`, `@code-reviewer` inside an OpenCode session. Confirm tool gating works as intended.

## 8. Install script — `scripts/install-opencode.{sh,ps1}`
- [ ] 8.1 Bash version: validates `opencode` and Bun are on PATH; copies `.opencode/` into project from a template; creates `~/.cortex/adapter-opencode.sock` (or HTTP bind reference); prints next-step instructions.
- [ ] 8.2 PowerShell mirror with the same UX on Windows. Reuse the polyglot pattern from `crates/cortex-adapter-claude-code/hooks/cortex-user-prompt.sh:11-15`.
- [ ] 8.3 Uninstall counterpart that removes only the files the install script wrote (never blanket-deletes `.opencode/`).

## 9. ADR-016
- [ ] 9.1 `rulebook_decision_create` ADR-016 — "OpenCode adapter via TS plugin + shared Rust daemon".
- [ ] 9.2 Trade-off documented: TS plugin couples to Bun runtime + plugin-API stability; gain: zero Rust changes for envelope capture, daemon stays the single source of truth.
- [ ] 9.3 Status `accepted` once §1 spike validates the plugin contract.

## 10. End-to-end smoke
- [ ] 10.1 Bring up `cortex-api`, `cortex-adapter-claude-code` (with HTTP listener), Synap, Vectorizer, Nexus, Meili.
- [ ] 10.2 Start an OpenCode session inside this repo with the new `opencode.json`.
- [ ] 10.3 Submit a prompt; invoke a tool; let a subagent run.
- [ ] 10.4 Inspect Synap `cortex.events.raw`: at least 4 envelopes with `tool: "opencode"` (Turn × 2, ToolCall × 1, AgentCall × 1).
- [ ] 10.5 Inspect the audit envelope from `cortex-api` for that session: `additionalContext` matches what the OpenCode TUI showed.
- [ ] 10.6 Toggle `CORTEX_OPENCODE_DISABLE=1`; re-run; confirm zero envelopes published (kill-switch parity with Claude Code).

## 11. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 11.1 Update or create documentation covering the implementation: new spec `docs/specs/20-opencode-adapter.md`, root `README.md` § "Hosts" listing both Claude Code and OpenCode, `crates/cortex-adapter-claude-code/README.md` cross-referencing spec 20, and `CHANGELOG.md` `Added` section noting the TS plugin + HTTP transport.
- [ ] 11.2 Write tests covering the new behavior: §2.6 socket/HTTP parity, §4.8 plugin event-mapper unit tests, §10 end-to-end smoke that publishes envelopes via the plugin to a fake Synap, plus a regression that `tool = "claude-code"` payloads still validate.
- [ ] 11.3 Run tests and confirm they pass: `cargo check -p cortex-adapter-claude-code && cargo clippy -p cortex-adapter-claude-code -- -D warnings && cargo test -p cortex-adapter-claude-code` clean, and `pnpm -C packages/cortex-opencode-plugin test && pnpm -C packages/cortex-opencode-plugin build` clean.
- [ ] 11.4 `rulebook_learn_capture` with title "OpenCode plugin event ordering and `tui.prompt.append` semantics".
