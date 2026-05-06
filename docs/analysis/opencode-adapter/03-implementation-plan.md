# 03 — Implementation plan

Phased plan to ship Cortex inside OpenCode at parity with Claude
Code. Each phase has a verifiable gate.

---

## Phase 0 — Spike (half day)

**Goal**: resolve the open questions before committing to the plugin
design.

- 0.1 Install OpenCode locally; create a throwaway project with an
  `opencode.json` registering `cortex-mcp-server` (stdio).
- 0.2 Confirm `cortex_query` shows up as an MCP tool in the OpenCode
  TUI and returns results.
- 0.3 Write a 50-line stub plugin that subscribes to
  `tool.execute.before`, `session.created`, `message.updated`,
  `tui.prompt.append`. Log every event to a file. Run a session;
  inspect the log to learn:
  - Which events fire on user prompt submission and in what order.
  - Whether `tool.execute.before` returning a Promise blocks the tool
    invocation until the Promise resolves.
  - Whether `tui.prompt.append` mutates the current prompt the LLM
    will see, or appends to a TUI buffer that ships next turn.
  - Whether `permission.asked` allows a plugin to deny.
- 0.4 Verify `session.idle` fires per-subagent or only on outer-turn
  end.

**Gate**: a one-page note in this directory documenting the answers,
plus a screen capture of the MCP tool listing inside OpenCode.

---

## Phase 1 — MCP + commands + agents (1 day, config-only)

**Goal**: model has tools and slash commands inside OpenCode.

- 1.1 Generate `opencode.json` from the existing `.mcp.json` and
  `.claude/settings.json`. Include both `cortex` and `rulebook` MCP
  entries under `mcp`.
- 1.2 Port `.claude/commands/*.md` → `.opencode/commands/*.md`.
  Wrap each existing body in `template:` frontmatter. Preserve
  `$ARGUMENTS` placeholders.
- 1.3 Port `.claude/agents/*.md` → `.opencode/agents/*.md`. For each
  agent:
  - Decide primary vs subagent (most are subagents).
  - Translate model/description/allowed-tools into OpenCode
    frontmatter (`model`, `temperature`, `permission`).
  - Convert permission lists to `allow` / `ask` / `deny` per tool
    category with glob patterns.
- 1.4 Add a `scripts/install-opencode.sh` that creates symlinks /
  copies under `.opencode/` and prints instructions for `opencode.json`.

**Gate**: in a fresh OpenCode session inside this repo, `cortex_query
"latest decision"` returns hits, and `/rulebook-task-list` works.

---

## Phase 2 — Adapter daemon HTTP listener (1 day)

**Goal**: the existing Rust daemon accepts hook posts from a web
client (the upcoming OpenCode plugin will be a TS module that runs
`fetch()` rather than `nc -U`).

- 2.1 Refactor `crates/cortex-adapter-claude-code/src/ipc.rs` so the
  binding is transport-agnostic. Today it listens on Unix socket /
  named pipe. Add an HTTP `POST /hook` endpoint with the same JSON
  payload shape.
- 2.2 New env var `CORTEX_ADAPTER_HTTP_BIND` (default
  `127.0.0.1:17004`); when set, the daemon serves both transports.
- 2.3 Tests: integration test for HTTP transport mirroring the
  existing socket test (`tests/dispatcher.rs`).
- 2.4 (Optional) Rename the crate to `cortex-adapter` with feature
  flags `claude-code` and `opencode`; defer if it complicates the
  migration.

**Gate**: `curl -X POST http://127.0.0.1:17004/hook -d @fixture.json`
publishes an envelope identical to the socket path.

---

## Phase 3 — TS plugin: envelope capture (2 days)

**Goal**: `@hivellm/cortex-opencode-plugin` package that captures
session activity and forwards it to the daemon.

### Package layout

```
packages/cortex-opencode-plugin/
├── package.json
├── tsconfig.json
├── src/
│   ├── index.ts          # Plugin export
│   ├── events.ts         # OpenCode event → adapter HookKind mapping
│   ├── client.ts         # HTTP client to cortex-adapter HTTP bind
│   ├── scope.ts          # cwd/worktree → repo slug derivation
│   └── config.ts         # env var loading, defaults
└── test/
    └── events.test.ts
```

### Plugin skeleton

```typescript
import type { Plugin } from "@opencode-ai/plugin"
import { postHook } from "./client"
import { mapEvent } from "./events"

export const CortexPlugin: Plugin = async ({ project, client, directory, worktree }) => {
  const ctx = { project, directory, worktree }
  return {
    "session.created":      async (e) => { await postHook("SessionStart", e, ctx) },
    "message.updated":      async (e) => { await postHook(mapEvent(e), e, ctx) },
    "tool.execute.before":  async (e) => { await postHook("PreToolUse", e, ctx) },
    "tool.execute.after":   async (e) => { await postHook("PostToolUse", e, ctx) },
    "session.idle":         async (e) => { await postHook("Stop", e, ctx) },
    "permission.asked":     async (e) => { await postHook("PermissionAsked", e, ctx) },
  }
}
```

### Tasks

- 3.1 Define `events.ts` mapping. Reuse the `HookKind` enum names from
  the Rust dispatcher so the daemon needs zero changes.
- 3.2 `client.ts` — `fetch` to `CORTEX_ADAPTER_HTTP_BIND`. Soft
  timeout 1500 ms (match daemon's pre-thinking budget). Fail-open on
  network errors.
- 3.3 `scope.ts` — same heuristics the existing adapter uses (cwd walk
  + git worktree → repo slug). Cache per-session.
- 3.4 Publish to a private npm registry or vendor as `.opencode/plugins/cortex.ts`
  for self-hosted use.

**Gate**: in an OpenCode session, after a user prompt + one tool call,
the daemon's `cortex.events.raw` Synap stream has at least 2 envelopes
with `tool: "opencode"`.

---

## Phase 4 — Pre-thinking injection (1-2 days)

**Goal**: model sees Cortex's bundle before it plans.

The exact mechanism depends on Phase 0 findings. Two candidate paths:

### Path A — `tui.prompt.append` (preferred if it works)

Plugin subscribes to the user-prompt event, calls
`postHook("UserPromptSubmit", …)` to fetch the bundle, then emits
`tui.prompt.append` with the bundle text marked as a system note.

```typescript
const bundle = await postHook("UserPromptSubmit", e, ctx)
if (bundle?.additionalContext) {
  await client.tui.prompt.append({ text: `\n\n<cortex>${bundle.additionalContext}</cortex>` })
}
```

### Path B — Priming message via SDK

If `tui.prompt.append` doesn't make the bundle visible to the
**current** model call, fall back to sending the bundle as a separate
priming message:

```typescript
await client.session.prompt({
  path: { id: sessionId },
  body: { parts: [{ type: "text", text: bundle.additionalContext }], system: true }
})
```

### Path C — Fallback via custom command

If neither A nor B works synchronously, register a `/cortex-prime`
slash command and have the user invoke it manually before complex
prompts. Strictly worse UX; only as fallback.

**Gate**: the bundle text appears in the same model call as the user
prompt (verified by checking the audit envelope from `cortex-api`).

---

## Phase 5 — Law-check (advisory or blocking) (1 day)

**Goal**: same law enforcement Claude Code's `PreToolUse` hook
provides today.

- 5.1 Plugin subscribes to `tool.execute.before`. Posts to the daemon's
  sync `/laws/check` endpoint. Receives verdict.
- 5.2 If OpenCode's plugin contract supports denying tool execution
  from `tool.execute.before` (Phase 0 finding): return `{ deny: true,
  reason }` (or whatever the plugin SDK accepts).
- 5.3 If not: emit `tui.toast.show` with the violation message
  (advisory only). Operator chooses to abort.
- 5.4 Wire `permission.asked` events too so high-severity laws can
  intercept the permission flow.

**Gate**: a tool invocation that violates a critical law is either
blocked (Path 5.2) or surfaces a toast (Path 5.3) within 500 ms.

---

## Phase 6 — Hardening + docs (1 day)

- 6.1 Test matrix: parity check between Claude Code and OpenCode for
  10 representative scenarios (prompt → response, tool call,
  subagent, decision lookup, law violation, scope override, …).
- 6.2 Update the project's main README + `crates/cortex-adapter-claude-code/README.md`
  with the OpenCode story and link to a new `crates/cortex-adapter-opencode/README.md`
  (or in the plugin package).
- 6.3 Add `.opencode/` to the same gitignore audit as `.claude/`.
- 6.4 Capture learnings (`rulebook_learn_capture`).
- 6.5 Create ADR `ADR-016 — OpenCode adapter via TS plugin + shared
  Rust daemon`.

**Gate**: a colleague following the README can install Cortex into a
fresh OpenCode setup in <10 min and see envelopes flowing.

---

## Schema and code deltas (cumulative)

| File | Change |
|------|--------|
| `crates/cortex-core/schemas/envelope.schema.json` | add `"opencode"` to `tool` enum |
| `crates/cortex-adapter-claude-code/src/ipc.rs` | add HTTP transport |
| `crates/cortex-adapter-claude-code/src/events.rs` | export `TOOL_OPENCODE` |
| `packages/cortex-opencode-plugin/` (new) | TS plugin package |
| `.opencode/commands/*.md` (new) | ports of `.claude/commands/*.md` |
| `.opencode/agents/*.md` (new) | ports of `.claude/agents/*.md` |
| `opencode.json` (new) | MCP + plugin + agent + permission config |
| `scripts/install-opencode.{sh,ps1}` (new) | install helper |
| `.gitignore` | ensure `.opencode/state/`, `.opencode/cache/` are ignored |

---

## Effort estimate

| Phase | Days | Cumulative |
|-------|------|-----------|
| 0 — Spike | 0.5 | 0.5 |
| 1 — MCP + commands + agents | 1 | 1.5 |
| 2 — HTTP listener | 1 | 2.5 |
| 3 — Plugin envelope capture | 2 | 4.5 |
| 4 — Pre-thinking injection | 1.5 | 6 |
| 5 — Law-check | 1 | 7 |
| 6 — Hardening + docs | 1 | 8 |

**Total: 8 working days** (4-6 days if Phase 0 reveals the plugin API
maps cleanly to today's hook contract; longer if injection requires
the SDK fallback).

---

## Risks

| Risk | Mitigation |
|------|-----------|
| OpenCode plugin events don't block tool calls → law-check is advisory only | Acceptable for v1; document the difference; revisit when OpenCode adds blocking semantics |
| `tui.prompt.append` doesn't reach the model call → bundle ineffective | Phase 4 Path B (SDK priming) as fallback; Phase 0 spike resolves this before plugin work |
| OpenCode's permission system has non-overlapping semantics with Claude Code | Document divergence in ADR-016; advisory-only for OpenCode |
| Plugin API is unstable and changes between OpenCode versions | Pin `@opencode-ai/plugin` peer dep; document the verified version; CI smoke test on plugin upgrade |
| Skills don't port → loss of feature parity | Re-author the few that matter as commands; drop the rest with a note in CHANGELOG |
