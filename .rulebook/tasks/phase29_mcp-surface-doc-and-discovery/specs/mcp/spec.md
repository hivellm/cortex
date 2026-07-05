# MCP tool discoverability and registry-sync enforcement

## ADDED Requirements

### Requirement: MCP tool registry is discoverable at runtime and checked against its doc
The Cortex MCP server SHALL expose a tool that returns its own complete,
current tool registry, and an automated check SHALL fail when the
documented registry and the live registry diverge.

#### Scenario: Doctor check reports registry divergence
Given a new MCP tool is added to the registry but the spec doc is not updated
When the doctor registry-sync check runs
Then it MUST report the divergence (missing/extra tool names) instead of passing silently

### Requirement: cortex_capabilities enumerates the live tool registry
The Cortex MCP server MUST expose a `cortex_capabilities` tool that
returns `{name, one_line_purpose, read_or_write}` for every tool
currently registered in `ToolRegistry::default_set()`, requiring no
input parameters.

#### Scenario: Agent self-discovers the full tool surface
Given an agent is connected to the Cortex MCP server and has not read any documentation or source code
When the agent calls `cortex_capabilities`
Then the response MUST list every tool in the live registry, and each entry MUST carry a non-empty `one_line_purpose` and a `read_or_write` value of either `read` or `write`
