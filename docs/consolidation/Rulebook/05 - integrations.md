# Rulebook Integrations

## Embedded in Every HiveLLM Project

Rulebook ships with every HiveLLM project via `npm install @hivehub/rulebook`. The `.rulebook/` directory and configuration files are generated on `init` and survive `update`.

## Integration with Cortex

Cortex consumes Rulebook data in three ways:

### 1. Task Management
- Cortex reads active tasks from `.rulebook/tasks/`
- MCP tool `rulebook_task_create` — create task in Cortex UI
- MCP tool `rulebook_task_archive` — archive completed work
- Task events published to Cortex for dashboard visibility
- Supports cross-project task coordination via `projectId` parameter

### 2. Persistent Memory
- Cortex queries memory via `rulebook_memory_search` (hybrid BM25+vector)
- Memory entries tagged by project + type (pattern, learning, decision)
- Consolidation tool exports memory index for Cortex ingestion
- Supports BM25 keyword search + HNSW vector similarity
- Three-layer search pattern: search → timeline → get

### 3. Knowledge Base
- Rulebook stores patterns, anti-patterns, learnings, decisions
- Cortex mirrors these into its own knowledge graph
- ADRs tracked via `rulebook_decision_create` with lifecycle (proposed → accepted → superseded)
- Cortex cross-references Rulebook decisions in node metadata

## AI Tool Integration

### Claude Code
```
CLAUDE.md (@import AGENTS.md + AGENTS.override.md)
  ↓
.claude/settings.json (hooks: SessionStart, UserPromptSubmit, Stop)
  ↓
.claude/rules/ (path-scoped rules + always-on rules)
  ↓
.claude/skills/ (workflow skills: handoff, ralph, terse-mode)
  ↓
.claude/agents/ (specialized agent prompts)
  ↓
.claude/commands/ (AI tool shortcuts)
```

### Cursor
- `.cursor/rules/` — auto-generated Cursor-specific rules
- Cursor loads from `AGENTS.md` + project overrides
- Same task/memory systems available via MCP

### Gemini, Copilot, Windsurf
- Tool-specific rule templates in `templates/cli/`
- Each tool gets its own rule file with same content, adapted for syntax
- MCP server available for all tools via stdio transport

## MCP Registration

One-time setup:
```bash
rulebook mcp init
```

Updates `.mcp.json`:
```json
{
  "mcpServers": {
    "rulebook": {
      "command": "node",
      "args": ["path/to/@hivehub/rulebook/dist/mcp/rulebook-server.js"],
      "stdio": "stdio"
    }
  }
}
```

Every tool loading `.mcp.json` gets 44+ Rulebook tools via stdio.

## Cortex Events & Webhooks

Rulebook publishes events to Cortex:
- `task.created` — new task created
- `task.archived` — task completed and archived
- `memory.saved` — context saved to persistent memory
- `memory.searched` — search query executed (for analytics)
- `decision.created` — new ADR recorded
- `learning.captured` — implementation insight saved

Cortex listens and updates its dashboard, knowledge graph, and audit log.

## Pre-Commit Hooks Integration

Rulebook installs git hooks that run before commit/push:

**Pre-commit**:
- Type-check (tsc, cargo check, mypy, etc.)
- Lint (eslint, clippy, pylint, etc.)
- Format (prettier, rustfmt, black, etc.)

**Pre-push**:
- Tests (vitest, cargo test, pytest, etc.)
- Coverage check (≥95% required)
- Security audit (npm audit, cargo audit, etc.)

Cortex monitors hook execution and reports failures.

## Skills System

Workflow skills extend Rulebook's behavior:

**Built-in Skills**:
- `handoff` — write session context at limits, restore on next session
- `ralph` — autonomous loop with multi-iteration task solving
- `terse-mode` — input/output compression with intensity levels

**Available Skills**:
- Language-specific (Rust, Python, TypeScript)
- Workflow-specific (Ralph, memory compression)
- Tool-specific (Cursor snippets, VS Code actions)

Enable with:
```bash
rulebook skill enable <skill-id>
```

## Cross-Project Workspace

For monorepos with 2+ projects:

```bash
rulebook workspace init
rulebook workspace add ./frontend
rulebook workspace add ./backend
rulebook mcp init --workspace
```

Single MCP server manages all projects:
- Each project has isolated task/memory
- `rulebook workspace search <query>` searches all projects
- `rulebook workspace tasks <project>` lists project tasks
- Cortex sees unified project view with per-project filtering

## Backwards Compatibility

Rulebook maintains compatibility with OpenSpec (predecessor):
- Task format: phase-prefixed IDs, proposal.md, tasks.md, specs/
- Spec language: SHALL/MUST + Given/When/Then scenarios
- Task lifecycle: pending → in-progress → complete/archived
- Migration: `rulebook update` auto-converts legacy OpenSpec tasks

## Environment Variables

Rulebook respects:
- `RULEBOOK_MCP_TIMEOUT_MS` — MCP tool timeout (default: 10000)
- `RULEBOOK_TERSE_MODE` — override terse intensity (off/brief/terse/ultra)
- `RULEBOOK_MEMORY_BACKEND` — force backend (better-sqlite3/sql.js)
- `RULEBOOK_PROJECT_ROOT` — explicit project directory
- `CORTEX_NEXUS_EXTERNAL_ID_IT` — smoke test flag (Cortex integration)
