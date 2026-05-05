# Rulebook Architecture

## System Components

### 1. Core CLI (`src/index.ts`, `src/cli/commands/`)
Entry point for `rulebook` command. Delegates to sub-commands:
- `init` — auto-detect stack, generate rules, install hooks
- `update` — pull latest rules, preserve project overrides
- `doctor` — 7 health checks (git, CI/CD, coverage, security, etc.)
- `task`, `memory`, `ralph`, `workspace` — domain-specific operations

### 2. Rule Generation Engine
- **Detector** (`src/core/detector.ts`) — identifies 28 languages, 17 frameworks, 13 MCP modules, 20 services
- **Template Engine** (`src/core/custom-templates.ts`) — generates language/framework-specific specs
- **Override Manager** (`src/core/override-manager.ts`) — merges base rules with project overrides

Outputs:
- `AGENTS.md` — base rules (regenerated on `update`)
- `AGENTS.override.md` — project-owned overrides (survives `update`)
- `CLAUDE.md` — Claude Code entry point with `@import` chain
- `.claude/rules/` — path-scoped rules (language-specific, always-on)

### 3. Task Management (`src/core/task-manager.ts`)
OpenSpec-compatible spec-driven development:
- Phase-prefixed task IDs: `phase11l_nexus-external-ids-migration`
- Mandatory structure: `proposal.md`, `tasks.md`, `specs/<module>/spec.md`
- Sequential enforcement (lowest-numbered phase first)
- Mandatory tail (docs + tests + verify) before archival
- Auto-validation of spec format (SHALL/MUST + Given/When/Then)

### 4. Persistent Memory System (`src/memory/`)
Local-only context that survives sessions:
- **Storage** — better-sqlite3 (native) with sql.js WASM fallback
- **Search** — hybrid BM25 keyword + HNSW vector (256-dim TF-IDF, no API calls)
- **Ranking** — Reciprocal Rank Fusion
- **Privacy** — auto-redact `<private>` tags

Three-layer pattern: `rulebook_memory_search` → `rulebook_memory_timeline` → `rulebook_memory_get`

### 5. MCP Server (`src/mcp/rulebook-server.ts`)
44+ tools via stdio transport:
- **Tasks** (7): CRUD, validate, archive, delete
- **Memory** (6): save, search, get, timeline, stats, cleanup
- **Ralph** (4): init, run, status, history
- **Skills** (6): list, enable, disable, search, validate, show
- **Workspace** (4): list, status, search, tasks
- **Knowledge/Decisions/Learnings** (10): add, list, show, update, supersede
- **Analysis** (3): create, list, show
- **Other** (4+): doctor, rules list, compress, evals

All tools accept optional `projectId` for workspace routing.

### 6. Skills System (`src/core/skills-manager.ts`)
Pluggable behavioral extensions:
- Workflow skills (handoff, ralph, terse-mode)
- Language-specific skills (Rust, Python, etc.)
- Enable/disable per project: `rulebook skill enable <skill-id>`

### 7. Ralph Autonomous Loop (`src/core/ralph-parallel.ts`)
Multi-iteration AI task solver:
- Reads PRD from tasks
- Fresh context per iteration
- 5 quality gates: type-check → lint → tests → coverage → security
- Parallel story execution
- Context compression (terse mode)
- Learning extraction
- Graceful pause/resume

### 8. VSCode Extension (`vscode-extension/`)
Real-time dashboard:
- Agents (team status, memory state)
- Tasks (progress bars, details, actions)
- Memory (stats, full-text search)
- Analysis (findings + execution plans)
- Doctor (7 health checks)
- Telemetry (MCP latency, success rates)

## Data Flow

```
User runs `rulebook init`
  ↓
Detector (auto-detect stack)
  ↓
Template Engine (generate rules)
  ↓
Rule files written (.rulebook/, .claude/, AGENTS.md, etc.)
  ↓
Git hooks installed (pre-commit, pre-push, SessionStart, UserPromptSubmit, Stop)
  ↓
MCP server registered in .mcp.json
  ↓
AI tool loads CLAUDE.md → @imports AGENTS.md → loads path-scoped rules
  ↓
AI operates under Rulebook rules + hooks enforce patterns
  ↓
Task/Memory/Ralph tools available via MCP
```

## Key Modules

| Module | Responsibility |
|--------|-----------------|
| `ConfigManager` | Load/save `.rulebook/rulebook.json` |
| `TaskManager` | CRUD tasks, validate specs, archive |
| `MemorySystem` | Persistent memory with BM25+HNSW search |
| `SkillsManager` | Load/enable/disable skills |
| `WorkspaceManager` | Multi-project coordination |
| `BackgroundIndexer` | Async file indexing for Cortex events |
| `RalphParallel` | Multi-iteration task solving |
| `VersionBumper` | Semantic version management |
| `ChangelogGenerator` | Conventional commit parsing |

## Technology Stack

- **Language**: TypeScript 5.3+
- **Runtime**: Node.js 20+
- **CLI Framework**: Commander.js
- **MCP SDK**: @modelcontextprotocol/sdk 1.22+
- **Database**: better-sqlite3 + sql.js WASM fallback
- **Search**: Custom BM25 + HNSW (256-dim TF-IDF)
- **UI**: Blessed (terminal) + VSCode WebView
- **Build**: TypeScript compiler + esbuild
- **Test**: Vitest with 95%+ coverage
