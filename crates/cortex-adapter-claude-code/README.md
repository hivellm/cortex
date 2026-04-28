# cortex-adapter-claude-code

> Spec: [`docs/specs/10-claude-code-adapter.md`](../../docs/specs/10-claude-code-adapter.md), [`docs/specs/12-pre-thinking-injection.md`](../../docs/specs/12-pre-thinking-injection.md)

Claude Code adapter for Cortex. Captures every meaningful interaction
inside a Claude Code session — user prompts, tool calls, agent calls,
sub-agent stops, session ends — and republishes them as Cortex
envelope events. Also injects pre-thinking context bundles into every
prompt so the model sees the relevant prior decisions, similar past
turns, and active laws before it plans.

```
Claude Code hook  ──┐
  UserPromptSubmit  │
  PreToolUse        ├──> cortex-adapter-claude-code ──> cortex.events.raw
  PostToolUse       │                ▲
  SubagentStop      │                │
  Stop              │                │ pre-thinking bundle
  Notification    ──┘                ▼
                                 cortex-api /v1/query (spec 11)
```

## Components

- **Library** (`src/lib.rs`) — hook envelope shapes, the `daemon`
  module that turns hook events into canonical envelopes, and the
  pre-thinking pipeline used by the `Stop` and `UserPromptSubmit`
  hooks. Pre-thinking calls into [`cortex-pre-thinking`](../cortex-pre-thinking/)
  rather than re-implementing the bundle assembly.
- **Binary `cortex-adapter-claude`** (`src/main.rs`) — the local
  daemon Claude Code's hooks `POST` to. Reads the adapter config from
  env vars + `~/.cortex/adapter-claude-code.toml`, publishes events
  to Synap, and turns sync-path hook calls into pre-thinking blocks
  the hook prints back to Claude Code.

## Hook contract

Every hook invocation receives JSON on stdin and writes JSON on
stdout. The adapter speaks the **camelCase** schema Claude Code
expects (e.g. `additionalContext` on `UserPromptSubmit`); fields
written in `snake_case` are silently ignored by Claude Code, which
is why this adapter is camelCase-strict.

The most relevant hooks today:

| Hook              | Adapter behaviour                                                                               |
|-------------------|--------------------------------------------------------------------------------------------------|
| `UserPromptSubmit`| Publishes a `Turn` envelope with `userMessage`; runs pre-thinking and returns `additionalContext`.|
| `PreToolUse`      | Publishes a `ToolCall` envelope with the tool input; pass-through (no blocking laws yet).         |
| `PostToolUse`     | Adds the tool output to the existing `ToolCall` event; routes `agent_call` separately.            |
| `SubagentStop`    | Closes an `AgentCall` envelope with the subagent's final output.                                  |
| `Stop`            | Emits the `assistant_message` half of the `Turn` so the user prompt and the model reply land in the same envelope. |
| `Notification`    | Captured for telemetry.                                                                          |

## Configuration

Set in `~/.claude/settings.json` (or per-project) so Claude Code
launches the daemon hooks. Example:

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "command": "cortex-adapter-claude hook user-prompt-submit" }],
    "PreToolUse":      [{ "command": "cortex-adapter-claude hook pre-tool-use" }],
    "PostToolUse":     [{ "command": "cortex-adapter-claude hook post-tool-use" }],
    "SubagentStop":    [{ "command": "cortex-adapter-claude hook subagent-stop" }],
    "Stop":            [{ "command": "cortex-adapter-claude hook stop" }]
  }
}
```

Daemon-side env vars:

| Variable                            | Default                       | Notes                                              |
|-------------------------------------|-------------------------------|----------------------------------------------------|
| `CORTEX_ADAPTER_SYNAP_URL`          | `http://127.0.0.1:15003`      | Synap base URL.                                    |
| `CORTEX_ADAPTER_API_URL`            | `http://127.0.0.1:15011`      | `cortex-api` base URL for pre-thinking lookups.    |
| `CORTEX_ADAPTER_PRE_THINKING_KB`    | `32`                          | Per-bundle cap (KB).                               |
| `CORTEX_ADAPTER_PRE_THINKING_TIMEOUT_MS` | `1500`                   | Soft cap; daemon emits an empty bundle past it.    |

## Pre-thinking

`cortex-adapter-claude-code` does **not** assemble bundles itself.
The bundle template, byte-budget enforcement, and per-section caps
live in [`cortex-pre-thinking`](../cortex-pre-thinking/). The adapter
only owns the scope-derivation heuristics from the user prompt + cwd
+ recent files, the round-trip to `cortex-api`, and the formatting of
the `additionalContext` block Claude Code injects.

## Tests

```bash
cargo test -p cortex-adapter-claude-code
```

Unit tests cover the hook contract (camelCase parsing, scope
derivation, recent-file TTL cache). Integration tests exercise the
daemon with an in-process Synap fake.

## Stability

Pre-1.0. The hook contract follows Claude Code's own schema, so this
crate moves whenever Claude Code's hooks change. Major-version drift
is documented in [`CHANGELOG.md`](CHANGELOG.md).
