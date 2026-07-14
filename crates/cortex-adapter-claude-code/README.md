# cortex-adapter-claude-code

> Spec: [`docs/specs/10-claude-code-adapter.md`](../../docs/specs/10-claude-code-adapter.md), [`docs/specs/12-pre-thinking-injection.md`](../../docs/specs/12-pre-thinking-injection.md)
>
> **Phase11w**: the same daemon serves OpenCode sessions via a new HTTP
> listener (`IpcBinding::Http(addr)`) under
> `CORTEX_ADAPTER_HTTP_BIND` (default `127.0.0.1:17004`). The TS
> plugin that posts to it ships in
> [`packages/cortex-opencode-plugin`](../../packages/cortex-opencode-plugin)
> and the host-agnostic adapter spec lives at
> [`docs/specs/23-opencode-adapter.md`](../../docs/specs/23-opencode-adapter.md).

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

### Env knobs (phase11w additions)

| Knob | Default | Purpose |
|------|---------|---------|
| `CORTEX_ADAPTER_HTTP_BIND` | unset (HTTP listener disabled) | When set, spawns the `IpcBinding::Http(addr)` listener alongside the primary socket/pipe binding. Default address `127.0.0.1:17004` when the knob is set without a value. The OpenCode TS plugin posts to `http://${bind}/hook` here. |

The HTTP listener serves `POST /hook` accepting the same `HookFrame`
JSON the socket / pipe paths accept; every frame funnels through the
same `Dispatcher::dispatch` entrypoint so envelope shapes are
byte-identical across transports.

### Claude Code hook registration

`cortex-adapter-claude install` patches `~/.claude/settings.json` to
launch the **`cortex-hook`** native binary on every Claude Code hook
event. The bin connects directly to the daemon's named pipe (Windows)
or Unix domain socket (Linux/macOS) and prints the daemon's reply on
stdout. Synchronous events (`UserPromptSubmit`, `PreToolUse`) wait
for the response so the bundle / verdict reaches Claude Code; the
remaining five events default to `--fire-forget` and disconnect after
the write so they don't pay the read-side latency.

Generated entries look like:

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "type": "command", "command": "cortex-hook UserPromptSubmit",  "owner": "cortex" }],
    "PreToolUse":       [{ "type": "command", "command": "cortex-hook PreToolUse",        "owner": "cortex" }],
    "PostToolUse":      [{ "type": "command", "command": "cortex-hook PostToolUse --fire-forget",   "owner": "cortex" }],
    "SubagentStop":     [{ "type": "command", "command": "cortex-hook SubagentStop --fire-forget",  "owner": "cortex" }],
    "Stop":             [{ "type": "command", "command": "cortex-hook Stop --fire-forget",          "owner": "cortex" }],
    "SessionStart":     [{ "type": "command", "command": "cortex-hook SessionStart --fire-forget",  "owner": "cortex" }],
    "Notification":     [{ "type": "command", "command": "cortex-hook Notification --fire-forget",  "owner": "cortex" }]
  }
}
```

The legacy `.sh` shims under `crates/cortex-adapter-claude-code/hooks/`
remain in tree as a Linux/macOS fallback for environments that do
not have `cortex-hook` on PATH. They are not registered by `install`
unless an operator restores them by hand.

Daemon-side env vars:

| Variable                            | Default                       | Notes                                              |
|-------------------------------------|-------------------------------|----------------------------------------------------|
| `CORTEX_ADAPTER_SYNAP_URL`          | `http://127.0.0.1:17003`      | Synap base URL.                                    |
| `CORTEX_ADAPTER_API_URL`            | `http://127.0.0.1:17000`      | `cortex-api` base URL for pre-thinking lookups.    |
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
