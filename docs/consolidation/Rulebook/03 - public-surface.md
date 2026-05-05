# Rulebook Public Surface

## CLI Commands

### Project Setup
```bash
rulebook init                    # Interactive setup
rulebook init --minimal          # Essentials only
rulebook init --lean             # AGENTS.md as <3KB index
rulebook init --light            # No quality enforcement
rulebook update                  # Update to latest rules
rulebook doctor                  # 7 health checks
rulebook validate                # Check standards compliance
rulebook health                  # Health score (0-100)
rulebook fix                     # Auto-fix common issues
```

### Task Management
```bash
rulebook task create <task-id>   # Create task (phase-prefixed)
rulebook task list               # List active tasks
rulebook task show <task-id>     # Show task details
rulebook task validate <task-id> # Validate spec format
rulebook task archive <task-id>  # Archive completed task
rulebook task delete <task-id>   # Delete permanently
```

### Memory & Knowledge
```bash
rulebook memory search <query>   # Hybrid BM25+vector search
rulebook memory save <text>      # Save context
rulebook memory get <id>         # Retrieve by ID
rulebook memory timeline <days>  # Window search
rulebook memory stats            # Database health
rulebook memory cleanup          # Evict old memories
rulebook knowledge add <text>    # Add pattern/anti-pattern
rulebook knowledge list          # View all patterns
rulebook knowledge show <id>     # Show pattern details
rulebook learn capture <text>    # Capture learning
rulebook learn list              # View learnings
rulebook learn promote <id>      # Promote to knowledge
rulebook decision create <text>  # Record decision
rulebook decision list           # View decisions
rulebook decision show <id>      # Show decision details
rulebook decision supersede <id> # Mark superseded
```

### Ralph Autonomous Loop
```bash
rulebook ralph init              # Generate PRD from tasks
rulebook ralph run               # Execute iteration loop
rulebook ralph run --max-iterations 10  # Cap iterations
rulebook ralph status            # Current progress
rulebook ralph history           # View iterations
rulebook ralph pause             # Gracefully pause
rulebook ralph resume            # Resume from pause
```

### Workspace
```bash
rulebook workspace init          # Create workspace config
rulebook workspace add <path>    # Add project
rulebook workspace list          # List all projects
rulebook workspace status        # Status with task counts
rulebook workspace search <query> # Search across projects
rulebook workspace tasks <project> # List project tasks
```

### Rules & Skills
```bash
rulebook rules list              # List rules by tier
rulebook rules project           # Project rules to all tools
rulebook skill list              # List available skills
rulebook skill add <skill-id>    # Enable skill
rulebook skill show <skill-id>   # Show skill details
rulebook skill enable <skill-id> # Activate skill
rulebook skill disable <skill-id> # Deactivate skill
rulebook skill search <query>    # Search skills
rulebook skill validate <skill-id> # Validate skill
```

### CI/CD & Quality
```bash
rulebook workflows               # Generate GitHub Actions
rulebook check-deps              # Audit dependencies
rulebook check-coverage          # Verify test coverage
rulebook version <major|minor|patch> # Bump semantic version
rulebook changelog               # Generate from git commits
rulebook compress <file>         # Input compression for memory files
rulebook compress --check <file> # Report compression ratio
```

## MCP Tools (44+)

### Task Management (7)
- `rulebook_task_create` — create task
- `rulebook_task_list` — list active tasks
- `rulebook_task_show` — show task details
- `rulebook_task_update` — update task status
- `rulebook_task_validate` — validate spec format
- `rulebook_task_archive` — archive completed task
- `rulebook_task_delete` — delete permanently

### Memory (6)
- `rulebook_memory_search` — hybrid BM25+vector search
- `rulebook_memory_save` — save context
- `rulebook_memory_get` — retrieve by ID
- `rulebook_memory_timeline` — window search
- `rulebook_memory_stats` — database health
- `rulebook_memory_cleanup` — evict old memories

### Skills (6)
- `rulebook_skill_list` — list available skills
- `rulebook_skill_show` — show skill details
- `rulebook_skill_enable` — activate skill
- `rulebook_skill_disable` — deactivate skill
- `rulebook_skill_search` — search skills
- `rulebook_skill_validate` — validate skill

### Knowledge & Learning (10)
- `rulebook_knowledge_add` — add pattern/anti-pattern
- `rulebook_knowledge_list` — list patterns
- `rulebook_knowledge_show` — show pattern details
- `rulebook_learn_capture` — capture learning
- `rulebook_learn_list` — list learnings
- `rulebook_learn_promote` — promote to knowledge
- `rulebook_decision_create` — record decision
- `rulebook_decision_list` — list decisions
- `rulebook_decision_show` — show decision details
- `rulebook_decision_supersede` — mark superseded

### Ralph (4)
- `rulebook_ralph_init` — generate PRD from tasks
- `rulebook_ralph_run` — execute iteration loop
- `rulebook_ralph_status` — current progress
- `rulebook_ralph_history` — view iterations

### Workspace (4)
- `rulebook_workspace_list` — list projects
- `rulebook_workspace_status` — status with task counts
- `rulebook_workspace_search` — search across projects
- `rulebook_workspace_tasks` — list project tasks

### Analysis (3)
- `rulebook_analysis_create` — create analysis
- `rulebook_analysis_list` — list analyses
- `rulebook_analysis_show` — show analysis details

### Other (4+)
- `rulebook_doctor_run` — run health checks
- `rulebook_rules_list` — list rules by tier
- `rulebook_compress` — compress memory files
- `rulebook_compress_list` — list candidates for compression
- `rulebook_evals_measure` — offline measurement
- `rulebook_evals_run` — live API regeneration
- `rulebook_indexer_status` — indexer health
- `rulebook_memory_session_end` — end-of-session summary
- `rulebook_session_init` — session initialization

All tools accept optional `projectId` for workspace routing.

## Configuration

Stored in `.rulebook/rulebook.json`:

```json
{
  "version": "5.5.2",
  "mode": "full",
  "features": {
    "mcp": true,
    "memory": true,
    "ralph": true,
    "multiAgent": true,
    "hooks": true,
    "telemetry": false
  },
  "terse": {
    "enabled": true,
    "defaultMode": "brief"
  }
}
```

## Generated Files

| File | Purpose |
|------|---------|
| `AGENTS.md` | Team-shared AI rules (regenerated on `update`) |
| `AGENTS.override.md` | Project overrides (survives `update`) |
| `CLAUDE.md` | Claude Code entry point with `@import` chain |
| `.claude/rules/` | Path-scoped rules (language-specific + always-on) |
| `.claude/settings.json` | Hooks and env vars for Claude Code |
| `.claude/commands/` | AI tool shortcuts (skill definitions) |
| `.claude/agents/` | Specialized agent prompts |
| `.claude/skills/` | Workflow skills (handoff, ralph, terse) |
| `.rulebook/specs/` | Detailed spec templates per language |
| `.rulebook/STATE.md` | Machine-written live status |
| `.rulebook/PLANS.md` | Session scratchpad |
| `.rulebook/tasks/` | Active task directories |
| `.rulebook/archive/` | Completed tasks with date stamps |
| `.rulebook/memory/` | Local persistent memory DB |
| `.rulebook/knowledge/` | Patterns and anti-patterns |
| `.rulebook/learnings/` | Implementation insights |
| `.rulebook/decisions/` | Architecture decision records |
