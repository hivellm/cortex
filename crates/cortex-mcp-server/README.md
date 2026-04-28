# cortex-mcp-server

> Spec: [`docs/specs/18-claude-code-plugin.md`](../../docs/specs/18-claude-code-plugin.md)

stdio JSON-RPC bridge that exposes Cortex to any MCP client (Claude
Code, Cursor's experimental MCP, Claude Desktop, etc.). Speaks the
Model Context Protocol with identifier-safe tool names and camelCase
schema fields so MCP clients accept the descriptors without dropping
fields.

```
MCP client (stdio)  ──▶ cortex-mcp-server ──▶ cortex-api /v1/query
                                       └─▶ cortex-pre-thinking
                                       └─▶ cortex-api /v1/status
```

## Tools

| Name               | Purpose                                                                                       |
|--------------------|-----------------------------------------------------------------------------------------------|
| `cortexQuery`      | Hybrid retrieval — same shape as `cortex-api /v1/query` (intent + scope + query → bundle).    |
| `cortexPreThinking`| Pre-formatted Markdown bundle for direct injection into a system prompt.                      |
| `cortexStatus`     | Health probe: surfaces Vectorizer / Nexus / Meili / Synap reachability + recent errors.       |

Tool descriptors are spec-18-compliant: dot-free identifier names,
camelCase JSON Schema fields, and a non-empty `description` on every
tool and parameter.

## Configuration

The server reads its target URLs from `cortex-api`-style env vars so
the same configuration surface drives both:

| Variable                     | Default                            |
|------------------------------|------------------------------------|
| `CORTEX_API_URL`             | `http://127.0.0.1:17000`           |
| `CORTEX_MCP_LOG_LEVEL`       | `info`                             |

## Run

The server is normally launched by an MCP client over stdio. For ad
hoc inspection:

```bash
cargo run --release -p cortex-mcp-server | jq
```

The plugin manifest under [`cortex-plugin/`](../../cortex-plugin/)
registers this binary with Claude Code; users add it via
`/plugin add cortex` (or the equivalent flow in their MCP client).

## Tests

```bash
cargo test -p cortex-mcp-server
```

Unit tests pin the JSON-RPC framing, the descriptor shape (every
tool name passes the spec's identifier regex), and the response
shape that `additionalContext` injection downstream expects.

## Stability

Pre-1.0. Tool names + schemas are the contract every MCP client
caches — renames force every client to reconnect, so they go through
the same review path as `cortex-core` envelope changes.
