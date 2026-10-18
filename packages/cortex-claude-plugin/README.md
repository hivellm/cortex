# Cortex for Claude Code

Version 0.4.0 uses one standalone `cortex` executable. Memory capture, search,
graph, revisions, MCP and lifecycle hooks run locally without a daemon, Docker,
a provider key or an API URL. Classification is optional and explicit.

Run `cortex install` to place the executable and bundled plugins under
`CORTEX_HOME` (default `~/.cortex`). Use the printed local marketplace path to
register this plugin in Claude Code. Keep its `bin` directory on the client PATH
for `cortex mcp`. Hook launchers also check `CORTEX_HOME/bin`, then
`USERPROFILE/.cortex/bin` on Windows or `HOME/.cortex/bin` elsewhere.

The plugin does not silently change client trust or settings. Enable the plugin
and approve hooks in the client. Codex uses `/hooks` to inspect hook trust.
Remove old adapter hooks before enabling overlapping lifecycle capture.

## Runtime contract

MCP starts `cortex mcp` over stdio. Five hooks invoke
`cortex hook --client claude-code --event EVENT`:
SessionStart, UserPromptSubmit, PostToolUse, Stop and SubagentStop.
Missing binaries or failed launches return `{}`. Hooks never start a daemon,
read transcripts, call a provider or repair an index. Initialize the store with
an explicit CLI command before expecting hook capture.

A repository needs `[cortex] id = "ExactRepositoryId"` in `cortex.toml`.
Scope matching is exact and case sensitive. Raw lifecycle observations do not
become accepted decisions automatically. Context includes source, authority,
record ID, revision and hash; delivery is not proof of model use or benefit.

The memory skill describes the eleven supported MCP names. Old law-enforcement,
compliance-audit and code-index endpoints are not part of this standalone core.
Classification tools only enqueue/list local jobs; explicit CLI processing may
contact the configured provider and requires authorization and credentials.

## Check

Run `cortex --help`, `cortex status --repository ExactRepositoryId`, and
`cortex mcp`. Inspect installed hooks in the client and make one real read with
`cortex_memory_read`. A successful read validates retrieval, not whether every
client lifecycle hook ran. Follow stderr diagnostics for explicit CLI failures.
