# 20 — OpenCode adapter (TS plugin + shared Rust daemon)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 10, 11, 12, 18

## Goal

Make Cortex work inside OpenCode (`opencode.ai`) at parity with Claude
Code: envelope capture, pre-thinking injection, MCP tools, custom
commands, and agents — all reachable from inside an OpenCode session
without losing institutional memory or governance.

This spec is the OpenCode counterpart to spec 10. The
`cortex-adapter-claude-code` daemon is shared verbatim; OpenCode
talks to it over a new HTTP listener instead of the Unix socket /
Windows named pipe spec 10 uses. The plugin layer is a thin
TypeScript module published as `@hivellm/cortex-opencode-plugin`.

## Scope

**In:**

- TypeScript plugin (`packages/cortex-opencode-plugin/`) that
  subscribes to OpenCode lifecycle events and posts envelopes to the
  daemon's HTTP listener.
- HTTP listener in the existing `cortex-adapter-claude-code` daemon
  (`IpcBinding::Http(addr)`), serving `POST /hook` with the same
  JSON shape the socket / pipe paths accept.
- `tool = "opencode"` enum addition on the envelope schema +
  `TOOL_OPENCODE` Rust constant.
- `opencode.json` project config wiring both `cortex` and `rulebook`
  MCP servers + the new plugin.
- `.opencode/commands/` and `.opencode/agents/` ports of the existing
  `.claude/` configs.
- Install / uninstall scripts at `scripts/install-opencode.{sh,ps1}`.

**Out:**

- Replacing Claude Code's socket / pipe transport — phase11w is
  additive; Claude Code keeps its existing capture path.
- Re-authoring `cortex-mcp-server` — same binary, same stdio
  transport, different config file location.
- Replacing `cortex-pre-thinking` — bundle assembly is unchanged.

## Hook contract

The OpenCode plugin subscribes to the lifecycle events documented in
the [Phase-0 spike](../analysis/opencode-adapter/00-spike.md) and maps
them to the canonical adapter `HookKind` strings the daemon already
understands:

| OpenCode event | Adapter `HookKind` | Notes |
|----------------|-------------------|-------|
| `session.created` | `SessionStart` | One per session. |
| `message.updated` (user, first text part) | `UserPromptSubmit` | Plugin de-dupes per message id. |
| `tool.execute.before` | `PreToolUse` | Plugin awaits the daemon's law-check response and may return `deny` to abort the tool call. |
| `tool.execute.after` | `PostToolUse` | Carries `result` + `error` from OpenCode's payload. |
| `permission.asked` | `PreToolUse` (advisory) | Plugin returns `"allow" / "ask" / "deny"` based on the daemon's law-check verdict. |
| `session.idle` (subagent boundary) | `SubagentStop` | Plugin uses `session.parent_id` to discriminate. |
| `session.idle` (outer boundary) | `Stop` | Fires once the outer turn finishes AND every spawned subagent has stopped. |

The wire shape on `POST /hook` is identical to the socket / pipe
path: a single `HookFrame` JSON object carrying `hook` (the
`HookKind` PascalCase string) + `session_id` + `cwd` + `payload`.
The response is the canonical `HookResponse` JSON the plugin reads
to extract the pre-thinking bundle (see §Sync paths).

## Envelope mapping

Every envelope the plugin produces carries `tool = "opencode"`. The
shape is otherwise identical to spec 10's mapping — same `kind`,
same `source.repo` / `branch` / `commit_sha` derivation (the
plugin's `scope.ts` mirrors the Rust adapter's `dispatcher::scope`
heuristics), same `context.extras` keys.

| OpenCode signal | Envelope `kind` | Extras |
|-----------------|-----------------|--------|
| `message.updated` (user) | `Turn` | `extras.opencode = { message_id }` |
| `tool.execute.after` | `ToolCall` | `extras.opencode = { tool_name, input_hash, output_size }` |
| `session.idle` (subagent) | `AgentCall` | `extras.opencode = { parent_session_id }` |

## Sync paths

The plugin issues two synchronous round-trips per turn:

1. **Pre-thinking** — on `message.updated` (user), the plugin posts a
   `UserPromptSubmit` frame and reads the response. The daemon's
   `dispatcher::dispatch` resolves the bundle by calling
   `cortex-pre-thinking` via `cortex-api` `/v1/query`. The response
   body's `hookSpecificOutput.additionalContext` carries the
   Markdown bundle. The plugin appends it through OpenCode's
   `tui.prompt.append` API (per the [spike answer
   c](../analysis/opencode-adapter/00-spike.md#c-does-tuipromptappend-mutate-the-current-model-call-or-buffer-next-turn)).

2. **Law check** — on `tool.execute.before` / `permission.asked`,
   the plugin posts a `PreToolUse` frame and reads
   `permissionDecision`. A `"deny"` verdict short-circuits the tool
   call (`tool.execute.before` rejects; `permission.asked` returns
   `"deny"`).

Both round-trips honour a 1500 ms wall budget client-side; on
timeout the plugin returns the fail-open default (empty bundle /
`allow`).

## Configuration

| Knob | Default | Where |
|------|---------|-------|
| `CORTEX_ADAPTER_HTTP_BIND` | `127.0.0.1:17004` | Daemon — gates whether the HTTP listener spawns. The TS plugin reads the same env to know where to POST. |
| `CORTEX_OPENCODE_DISABLE` | unset (active) | Plugin — kill-switch. When `1` the plugin reports `init=false` and never publishes. |
| `CORTEX_OPENCODE_PRE_THINKING_KB` | `12` | Plugin — soft cap on the bundle size the plugin appends to the TUI (the daemon's `cortex_config::PreThinkingConfig.budget_kb` is the hard cap). |
| `CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS` | `1500` | Plugin — wall budget for the daemon round-trip. |

The HTTP listener is loopback-only by default (`127.0.0.1`). The bind
addr is operator-configurable for container scenarios where the
plugin lives in another network namespace.

## Plugin contract

`packages/cortex-opencode-plugin/` ships as
`@hivellm/cortex-opencode-plugin` on npm.

```ts
import { type Plugin } from "@opencode-ai/plugin";
export const CortexPlugin: Plugin = async (ctx) => {
  // subscribes to session.created, message.updated,
  // tool.execute.before, tool.execute.after, permission.asked,
  // session.idle; posts to CORTEX_ADAPTER_HTTP_BIND/hook.
};
export default CortexPlugin;
```

The plugin owns:

- Event → `HookKind` mapping (`src/events.ts`).
- Daemon HTTP client with 1500 ms timeout + fail-open (`src/client.ts`).
- Repo / branch / commit-sha derivation cached per session
  (`src/scope.ts`).
- Env-knob loading (`src/config.ts`).
- Subagent boundary discrimination via `session.parent_id`.

## Stability

- **Wire shape on `POST /hook`** is the spec-10 `HookFrame` JSON.
  Additive fields are allowed on the payload; removing or
  re-typing any existing field is a breaking change.
- **`HookResponse`** shape is the spec-10 contract verbatim. The
  plugin reads `hookSpecificOutput.additionalContext` and
  `permissionDecision` only — extra fields are ignored.
- **`tool = "opencode"`** is the only new envelope-schema enum
  value. Removing it is a breaking change.
- **Plugin runtime version** is pinned in
  `packages/cortex-opencode-plugin/package.json` as a peer dep on
  `@opencode-ai/plugin`. A major-version bump on the plugin runtime
  may require a follow-up; minor / patch bumps stay compatible.

## References

- Spec 10 — Claude Code adapter (sibling capture path).
- Spec 11 — Query API (consumed by the daemon's sync path).
- Spec 12 — Pre-thinking injection (the bundle the plugin appends).
- Spec 18 — Claude Code plugin (MCP server reused verbatim).
- Phase 0 spike — [`docs/analysis/opencode-adapter/00-spike.md`](../analysis/opencode-adapter/00-spike.md).
- OpenCode docs: https://opencode.ai/docs/ (plugin contract).
- `@opencode-ai/plugin` types: published TypeScript declarations.
