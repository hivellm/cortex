# MCP/API tool registry spec doc diverges from runtime registry with no automated check

**Category**: observability
**Tags**: cortex, mcp, observability, analysis:cortex-platform-2026-07

## Description

docs/specs/20-mcp-tool-surface.md documented exactly 7 Cortex MCP tools; the runtime ToolRegistry::default_set() actually registers 37. No CI check ever compared the documented table's row count against the live registry's length, so 30 tools shipped with zero corresponding documentation. General lesson: any generated or registered surface (API routes, feature flags, MCP tools, CLI subcommands) needs an automated coherence check against its own doc, or the doc silently becomes fiction the moment someone adds an entry without updating the table.

## When to Use

Any time a spec/doc claims to enumerate a registry, list, or surface that is also defined in code (tool lists, route tables, feature-flag catalogs).
