# MCP server contract spec

## MODIFIED Requirements

### Requirement: MCP tool names follow the spec identifier rule
The `cortex-mcp-server` SHALL use tool names matching `[a-zA-Z0-9_-]+` per the MCP 2024-11-05 spec. Names MUST NOT contain `.` or any character outside that set.

#### Scenario: tools/list response uses underscore-separated names
Given a Claude Code client connects to `cortex-mcp-server` via stdio
When the client sends `tools/list`
Then the response MUST contain a tool named `cortex_query` (not `cortex.query`)
And the response MUST contain a tool named `cortex_pre_thinking` (not `cortex.pre_thinking`)
And the response MUST contain a tool named `cortex_status` (not `cortex.status`)

### Requirement: MCP descriptors use camelCase schema field
The `cortex-mcp-server` SHALL emit `inputSchema` (camelCase) in tool descriptors per the MCP 2024-11-05 spec. Snake_case `input_schema` is forbidden.

#### Scenario: descriptor carries inputSchema
Given the server returns a tools/list response
When the client parses each descriptor
Then every descriptor MUST contain an `inputSchema` JSON key
And no descriptor MAY contain an `input_schema` JSON key
And if `outputSchema` is present, it MUST also be camelCase

### Requirement: Internal callers reference tools by the new names
Any caller that addresses Cortex MCP tools by name (slash commands, agents, scripts, docs) SHALL use the underscore form. Hard-coded references to `cortex.query` / `cortex.pre_thinking` / `cortex.status` are forbidden.

#### Scenario: cortex-plugin slash commands updated
Given the `cortex-plugin/commands/cortex-query.md` slash command instructs the model to invoke the MCP tool
When the command is rendered
Then the prompt MUST reference `cortex_query` (or the fully-qualified `mcp__cortex__cortex_query`), never `cortex.query`
