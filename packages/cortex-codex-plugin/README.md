# Cortex for Codex

Version 0.4.1 uses one standalone `cortex` executable. Memory capture, search,
graph, revisions, MCP and lifecycle hooks run locally without a daemon, Docker,
a provider key or an API URL. Classification is optional and explicit.

Run `cortex install` to place the executable and bundled plugins under
`CORTEX_HOME` (default `~/.cortex`). Use the printed local marketplace path to
register this plugin in Codex. Keep its `bin` directory on the client PATH
for `cortex mcp`. Hook launchers also check `CORTEX_HOME/bin`, then
`USERPROFILE/.cortex/bin` on Windows or `HOME/.cortex/bin` elsewhere.

The plugin does not silently change client trust or settings. Enable the plugin
and approve hooks in the client. Codex uses `/hooks` to inspect hook trust.
Remove old adapter hooks before enabling overlapping lifecycle capture.

## Runtime contract

MCP starts `cortex mcp` over stdio. Five hooks invoke
`cortex hook --client codex --event EVENT`:
SessionStart, UserPromptSubmit, PostToolUse, Stop and SubagentStop.
Missing binaries or failed launches return `{}`. Hooks never start a daemon,
read transcripts, call a provider or repair an index. Initialize the store with
an explicit CLI command before expecting hook capture.

A repository needs `[cortex] id = "ExactRepositoryId"` in `cortex.toml`.
Scope matching is exact and case sensitive. Raw lifecycle observations do not
become accepted decisions automatically. Context includes source, authority,
record ID, revision and hash; delivery is not proof of model use or benefit.

The memory skill describes the fifteen supported MCP names. Old law-enforcement,
compliance-audit and code-index endpoints are not part of this standalone core.

Version 0.4.1 adds skill guidance for local audit list/get/metrics/retention.
Audit inspection preserves exact historical references and separates prepared
context, related actions, captured citation evidence and unknown attribution.
It does not claim causal model use or productivity. Retention previews by
default; explicit apply archives visibility without deleting canonical sources.
Hook definitions are unchanged. After installing the matching executable and
plugin bundle, refresh the client plugin cache to load the updated skills.
Classification tools only enqueue/list local jobs; explicit CLI processing may
contact the configured provider and requires authorization and credentials.

## Check

Run `cortex --help`, `cortex status --repository ExactRepositoryId`, and
`cortex mcp`. Inspect installed hooks in the client and make one real read with
`cortex_memory_read`. A successful read validates retrieval, not whether every
client lifecycle hook ran. Follow stderr diagnostics for explicit CLI failures.
