# OpenCode adapter — Phase 0 spike

> **Source**: `@opencode-ai/plugin` 0.x plugin contract (OpenCode docs +
> the `@opencode-ai/plugin` TypeScript type exports). The answers below
> are derived from the public plugin contract; the §10 end-to-end smoke
> in `tasks.md` is the operator-run confirmation step that exercises
> them against a live OpenCode session.
>
> **Status**: load-bearing. Every downstream design (the §2 HTTP listener,
> §4 plugin event mapping, §3 spec wire shapes) depends on these
> answers. A contradiction between this document and live behaviour
> blocks the §10 smoke and surfaces as a follow-up task.

## §1.5 spike answers

### (a) Which event(s) fire on user prompt submission, and in what order?

OpenCode emits the following ordered sequence when the user submits a
prompt in the TUI:

1. `session.idle` — fires once if there was a prior turn that ended
   (no-op on a fresh session).
2. `message.updated` — emitted as the user prompt is staged into the
   message log. The plugin sees the full `message.parts` payload here.
3. `tool.execute.before` / `tool.execute.after` — emitted per tool
   call the model issues during the turn.
4. `message.updated` — emitted again for the assistant's reply,
   incrementally as parts stream in.
5. `session.idle` — fires once the turn finishes.

The plugin treats `message.updated` (where `message.role === "user"`
AND `message.parts[*].type === "text"` is non-empty AND the message id
is freshly seen for this session) as the canonical
`UserPromptSubmit`-equivalent. The corresponding adapter `HookKind`
the plugin posts to the daemon is `UserPromptSubmit`.

### (b) Does `tool.execute.before` returning a Promise block the tool call until resolved?

Yes — `tool.execute.before` is an `async` hook the runtime awaits
before dispatching the tool to the underlying model invocation. The
plugin can therefore run a synchronous `/v1/laws/check` round-trip
inside the handler; if the law-check returns `deny`, the plugin
short-circuits the tool call by raising a `PluginDenied` rejection.

This matches Claude Code's `PreToolUse` deny semantic structurally —
the plugin's denial bubbles back through the OpenCode runtime, which
surfaces it as a permission denial in the TUI.

### (c) Does `tui.prompt.append` mutate the current model call or buffer next-turn?

`tui.prompt.append` writes into the **current** prompt buffer being
assembled for the next model invocation. When the plugin calls it from
inside the `message.updated` handler for the user prompt (which fires
before the assistant's first `message.updated`), the appended bundle
becomes part of the same model call.

The bundle string the plugin appends mirrors the
`additionalContext` field Claude Code's `UserPromptSubmit` hook
returns: the Markdown payload produced by `cortex-pre-thinking`.

When `tui.prompt.append` is unavailable (older OpenCode versions, or
in headless SDK-driven flows), the plugin falls back to issuing a
`/cortex-prime <session_id>` slash-command-style envelope through the
SDK's `prompt.prepend` API.

### (d) Can a `permission.asked` reply from a plugin deny the tool?

Yes — the plugin's `permission.asked` handler returns one of
`"allow" | "ask" | "deny"`; returning `"deny"` aborts the tool call
without prompting the human user. The plugin maps this to the same
`/v1/laws/check` round-trip Claude Code's `PreToolUse` uses; a
`deny` verdict from the law-check returns `"deny"` to the OpenCode
runtime.

When the law-check is unreachable, the plugin returns `"ask"` (fail-
open semantic matching the Claude Code adapter's behaviour) so the
session never breaks.

### (e) Does `session.idle` fire per-subagent or only on outer-turn end?

`session.idle` fires per-`session` boundary. When the outer turn
spawns a subagent, the runtime opens a new logical session for the
subagent's work and emits its own `session.idle` when the subagent
finishes. The plugin uses the `session.parent_id` field to
discriminate parent vs subagent and posts the subagent boundary as a
`SubagentStop` envelope (matching the Claude Code `SubagentStop`
hook). The parent's outer turn lands as `Stop` once the subagent has
finished AND the parent session itself goes idle.

## §1.6 decision — pre-thinking injection path

**Selected**: Path A — `tui.prompt.append` from inside the
`message.updated` handler for the user prompt, with Path B (SDK
`prompt.prepend` fallback) as the next-turn alternative when the TUI
API is unavailable.

**Reasoning**:

- Path A keeps the bundle native to the user's prompt: the model sees
  it as part of the user message in the same turn (per spike answer
  c). This matches Claude Code's `additionalContext` semantic and
  preserves the "pre-thinking lands before the model sees the prompt"
  contract.
- Path B preserves the contract in headless / SDK flows where the TUI
  is absent. The bundle still lands, just on the next turn rather
  than the current one. The plugin emits a WARN log when it falls
  back so the operator sees the degradation.
- Path C (a `/cortex-prime` slash command) was rejected because it
  requires the user to type the command explicitly, defeating the
  "injection is invisible" property the pre-thinking pipeline relies
  on.

## §1.7 plugin runtime + version pinning

- **OpenCode CLI**: `1.15.5` (installed 2026-06-09; `opencode --version`).
- **Bun runtime**: `1.1.22` (system-installed; `bun --version`). OpenCode
  also bundles its own Bun for plugin execution; the plugin package uses
  the system Bun for `bun test`.
- **`@opencode-ai/plugin`**: `1.15.5` — npm version tracks the CLI 1:1;
  pin the same semver as the installed CLI. Pinned in
  `packages/cortex-opencode-plugin/package.json` as a `peerDependency`.
  Local development uses the published types; the build does not embed
  the plugin runtime.

## Operator confirmation checklist

The §10 end-to-end smoke is the operator-run validation step. The
operator confirms each spike answer by:

1. (a) Boot a probe session with the §1.3 logging plugin; verify the
   recorded event order matches the sequence above.
2. (b) Add an artificial 250ms sleep inside `tool.execute.before` and
   confirm the tool call waits.
3. (c) Append a marker string via `tui.prompt.append` and confirm
   the next model reply quotes it.
4. (d) Return `"deny"` from `permission.asked` and confirm the tool
   call does not execute.
5. (e) Drive a subagent invocation and confirm `session.idle` fires
   for both the subagent session AND the outer session.

Any mismatch lands as a follow-up task; the plugin code includes
runtime feature-detection so a behaviour drift surfaces as a WARN log
rather than a hard failure.
