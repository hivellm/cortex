# @hivellm/cortex-opencode-plugin

OpenCode plugin bridging session lifecycle events to the
`cortex-adapter-claude-code` daemon. Ships envelope capture +
pre-thinking injection + law-check parity with the Claude Code
adapter (spec 10), targeted at the OpenCode host.

Full spec: [`docs/specs/20-opencode-adapter.md`](../../docs/specs/20-opencode-adapter.md).

## Install

```bash
npm i -D @hivellm/cortex-opencode-plugin
# or via opencode.json:
#   "plugin": ["@hivellm/cortex-opencode-plugin"]
```

The plugin POSTs to the daemon's HTTP listener. Start the daemon with
the HTTP transport enabled:

```bash
CORTEX_ADAPTER_HTTP_BIND=127.0.0.1:17004 cortex-adapter-claude daemon
```

## Configuration

| Env | Default | Purpose |
|-----|---------|---------|
| `CORTEX_ADAPTER_HTTP_BIND` | `127.0.0.1:17004` | Daemon hook endpoint the plugin POSTs to. |
| `CORTEX_OPENCODE_DISABLE` | unset | Set to `1` to disable the plugin (kill-switch). |
| `CORTEX_OPENCODE_PRE_THINKING_KB` | `12` | Soft cap on the bundle size the plugin appends to the TUI. |
| `CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS` | `1500` | Wall budget for the daemon round-trip. |

## Build + test

```bash
pnpm install
pnpm test
pnpm build
```
