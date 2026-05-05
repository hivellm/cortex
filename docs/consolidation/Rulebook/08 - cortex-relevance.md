# Rulebook — Cortex Relevance

## Laws & Overrides as Context

Rulebook generates **tier-1 prohibitions** and **editing discipline rules** that become part of every project's CLAUDE.md. Cortex should ingest and understand these laws because they define how AI agents operate.

### Laws Cortex Should Sync

1. **No shortcuts, stubs, placeholders** — AI must implement completely
2. **No destructive git ops** — preserve uncommitted work
3. **No deferred tasks** — checklist items must complete or defer explicitly
4. **Sequential task execution** — no cherry-picking from tasks.md
5. **Research before implementing** — state KNOW/DON'T KNOW before coding

These laws flow into Cortex's agent instructions to maintain consistency across sessions.

### Project-Specific Overrides

Each project's `AGENTS.override.md` contains **project-specific laws** that override default Rulebook rules. Examples:
- `LAW-CORTEX-001` — strict task-sequence enforcement
- Custom quality gates or approval processes
- Language-specific prohibitions

Cortex should:
1. Read `AGENTS.override.md` at session start
2. Enforce these laws before delegating work
3. Include them in agent delegation context

## Knowledge Base Integration

### Memory as Cortex Input

Rulebook's persistent memory contains:
- **Patterns** — reusable solutions ("JWT auth for microservices")
- **Anti-patterns** — what not to do ("Don't hardcode secrets in config")
- **Learnings** — implementation insights ("TF-IDF vectors are deterministic")
- **Decisions** — ADRs with lifecycle ("HNSW chosen for memory search")

Cortex should:
1. Periodically sync `.rulebook/knowledge/`, `.rulebook/learnings/`, `.rulebook/decisions/`
2. Index them in Cortex's own knowledge graph
3. Reference them when suggesting implementations
4. Cross-link Cortex nodes with Rulebook ADRs

### Search Integration

Cortex can query Rulebook memory directly:
```bash
# Via MCP
rulebook_memory_search "authentication" --type pattern

# Via CLI
npx rulebook memory search "authentication"
```

### Timeline Queries

For session continuity, Cortex can retrieve recent context:
```bash
# What was done in past 7 days?
rulebook memory timeline 7
```

## Decision Records as Cortex Audit Trail

Rulebook's ADR system (`.rulebook/decisions/`) tracks architectural choices:
- Decision statement
- Lifecycle (proposed → accepted → superseded)
- Timestamp + author + rationale

Cortex should:
1. Sync ADRs to its own decision log
2. Cross-reference when suggesting major changes
3. Create ADR when Cortex makes architectural decisions
4. Mark superseded decisions (never delete, preserve history)

Example ADRs in Rulebook itself:
- "Use HNSW over FAISS for memory search" — accepted
- "Hybrid BM25+HNSW over pure LLM embeddings" — accepted
- "Phase-numbered task sequences" — accepted

## Task Pipeline as Cortex Source of Truth

Rulebook's task system (`.rulebook/tasks/`) is spec-driven:
- Each task has proposal + checklist + detailed specs
- Sequential enforcement (phase-numbered)
- Archival with date stamps

Cortex should:
1. Read active tasks from all projects' `.rulebook/tasks/`
2. Display task progress in its dashboard
3. Create/update tasks via `rulebook_task_*` MCP tools
4. Archive via MCP when work completes

### Cross-Project Task Visibility

In workspaces:
```bash
rulebook workspace tasks <project>    # List project tasks
rulebook workspace search <query>     # Search all projects
```

Cortex should expose unified task view with per-project filtering.

## Handoff System Integration

When context limits approached:
1. **SessionStart hook** — checks context usage percentage
2. **Trigger** — if >75%, suggest `/handoff`; if >90%, mandate
3. **Write** — `rulebook_memory_session_end` saves state to `.rulebook/handoff/_pending.md`
4. **Restore** — next session's SessionStart loads pending state

Cortex should:
1. Monitor handoff signals from Rulebook hooks
2. Alert user when approaching context limits
3. Offer `/handoff` skill or auto-trigger at 90%
4. Preserve pending context for next session

## MCP Tool Catalog as Cortex Operations

Rulebook exposes 44+ MCP tools that Cortex can call:

**Critical for Cortex**:
- `rulebook_task_create`, `rulebook_task_archive` — manage tasks
- `rulebook_memory_search`, `rulebook_memory_save` — persistent context
- `rulebook_decision_create`, `rulebook_decision_supersede` — track decisions
- `rulebook_workspace_search` — cross-project queries

**Supplementary**:
- `rulebook_skill_enable/disable` — enable/disable workflow features
- `rulebook_knowledge_add` — capture patterns
- `rulebook_learn_capture` — record learnings

All tools support optional `projectId` for workspace routing.

## Quality Gates Alignment

Rulebook enforces 5 quality gates (type-check → lint → tests → coverage → security). Cortex should:
1. **Monitor pre-commit/pre-push hook output** — health indicators
2. **Fail fast** — stop work if type-check fails (diagnostic-first)
3. **Coverage reporting** — surface <95% coverage as blocker
4. **Security audit** — block on critical vulns (npm audit, cargo audit, etc.)

### Ralph Integration

Ralph (autonomous loop) runs these gates in fresh context per iteration. Cortex should:
1. Trigger Ralph when task complexity is high
2. Monitor Ralph iterations in `.rulebook/ralph/history/`
3. Extract learnings after each iteration
4. Pause/resume Ralph if user intervenes

## Session Continuity Handoff

Rulebook's handoff system writes to `.rulebook/handoff/_pending.md`:
```markdown
# Session Handoff

**Active Task**: phase11l_nexus-external-ids-migration
**Progress**: §2.3 complete, §3 in progress
**Files Touched**: src/api/cortex.ts, src/storage.rs
**Decisions**: Used HNSW for search
**Next Steps**: Complete §3, run full test suite
```

Cortex should:
1. Read `_pending.md` at session start
2. Load task details via `rulebook_task_show`
3. Restore memory context from timeline
4. Resume work from documented checkpoint

## Compression for Large Projects

For projects with extensive memory, Rulebook offers input compression:
```bash
rulebook compress --check CLAUDE.md    # Report ratio
rulebook compress CLAUDE.md            # Rewrite + backup
```

Cortex should:
1. Monitor memory DB size
2. Suggest compression if >50MB
3. Run monthly maintenance cleanup

## Data Flow: Rulebook → Cortex

```
.rulebook/tasks/
  ↓ (rulebook_task_list)
Cortex Dashboard (task progress)
  ↓ (rulebook_task_create/archive)
Back to .rulebook/tasks/

.rulebook/memory/
  ↓ (rulebook_memory_search)
Cortex Knowledge Graph
  ↓ (cross-references in node metadata)
Back to .rulebook/memory/

.rulebook/decisions/
  ↓ (rulebook_decision_list)
Cortex Audit Trail
  ↓ (rulebook_decision_create/supersede)
Back to .rulebook/decisions/

.rulebook/PLANS.md
  ↓ (session scratchpad)
Cortex Context Injection
  ↓ (updates after session)
Back to .rulebook/PLANS.md
```

## Critical Rule: Keep Memory Fresh

Rulebook memory is **local-only, zero API cost**. Cortex should:
1. **Periodically sync** (daily) — ingest new memories
2. **Never delete** — use cleanup for old entries only
3. **Cross-link** — reference Rulebook IDs in Cortex nodes
4. **Respect privacy** — auto-redaction for `<private>` tags
