# Rulebook Operational Guide

## Installation

### Global CLI
```bash
npm install -g @hivehub/rulebook@latest
rulebook init
```

### Project-Local (Recommended)
```bash
npm install --save-dev @hivehub/rulebook@latest
npx rulebook init
```

### Minimum Node.js Version
Node.js 20.0.0+ required (check: `node --version`)

## First-Time Setup

```bash
# 1. Initialize project (auto-detects stack)
npx rulebook init

# 2. Verify health
npx rulebook doctor

# 3. Register MCP with your editor
npx rulebook mcp init

# 4. Optional: Enable skills
npx rulebook skill enable handoff
npx rulebook skill enable ralph
```

Generated files:
- `.rulebook/` — configuration + memory + tasks
- `.claude/` — Claude Code hooks + rules
- `.cursor/` — Cursor-specific rules
- `AGENTS.md` — team-shared rules
- `AGENTS.override.md` — your overrides
- `CLAUDE.md` — Claude Code entry point

## Common Workflows

### Create & Track a Task
```bash
# Create task with interactive prompts
npx rulebook task create phase1_add-auth

# List active tasks
npx rulebook task list

# Show task details
npx rulebook task show phase1_add-auth

# Validate spec format before implementing
npx rulebook task validate phase1_add-auth

# Update status as you progress
npx rulebook task update phase1_add-auth --status in-progress

# Mark item complete in tasks.md
# (edit .rulebook/tasks/phase1_add-auth/tasks.md manually)

# Archive when done (enforces docs + tests + verify)
npx rulebook task archive phase1_add-auth
```

### Use Persistent Memory
```bash
# Save context (captures for next session)
npx rulebook memory save "Chose JWT over sessions for auth"

# Search memory
npx rulebook memory search "authentication approach"

# View memory stats
npx rulebook memory stats

# Get full record by ID
npx rulebook memory get <id>

# View timeline window (past 30 days)
npx rulebook memory timeline 30
```

### Capture Learnings & Patterns
```bash
# Record implementation insight
npx rulebook learn capture "JWT tokens simpler than session store for microservices"

# List all learnings
npx rulebook learn list

# Promote learning to reusable pattern
npx rulebook learn promote <id>

# View patterns (before implementing similar feature)
npx rulebook knowledge list
```

### Record Architecture Decisions
```bash
# Create new ADR
npx rulebook decision create "Use HNSW for vector search (not FAISS)"

# List all decisions
npx rulebook decision list

# Mark decision superseded
npx rulebook decision supersede <id> --reason "Replaced by Redis cluster strategy"
```

### Run Ralph Autonomous Loop
```bash
# Generate PRD from active tasks
npx rulebook ralph init

# Execute iteration loop (up to 10 iterations)
npx rulebook ralph run --max-iterations 10

# Check progress
npx rulebook ralph status

# View past iterations
npx rulebook ralph history

# Pause gracefully (saves state)
npx rulebook ralph pause

# Resume from pause
npx rulebook ralph resume
```

### Multi-Project Workspace
```bash
# Initialize workspace (for monorepo)
npx rulebook workspace init

# Add projects
npx rulebook workspace add ./frontend
npx rulebook workspace add ./backend

# List all projects
npx rulebook workspace list

# View status across projects
npx rulebook workspace status

# Search across all projects
npx rulebook workspace search "authentication"

# List tasks in specific project
npx rulebook workspace tasks frontend
```

## Configuration

Edit `.rulebook/rulebook.json`:

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
  },
  "memory": {
    "backend": "better-sqlite3",
    "evictionDays": 180
  }
}
```

### Disable Features
```json
{
  "features": {
    "hooks": false,      // Skip pre-commit/pre-push
    "ralph": false,      // Disable autonomous loop
    "telemetry": false   // No metrics collection
  }
}
```

## Quality Gates

Pre-commit hook validates:
```bash
npm run type-check    # ✓ Must pass
npm run lint          # ✓ Must pass (zero warnings)
npm run format        # ✓ Auto-fixes style
```

Pre-push hook validates:
```bash
npm test              # ✓ Must pass (100%)
npm run test:coverage # ✓ Coverage ≥95%
npm audit             # ✓ No critical vulns
```

Manual run:
```bash
npx rulebook health    # Score 0-100
npx rulebook doctor    # 7 health checks
npx rulebook fix       # Auto-fix common issues
```

## Troubleshooting

### MCP Server Won't Start
```bash
# Check if .rulebook exists
ls .rulebook

# Reinitialize MCP config
npx rulebook mcp init

# Verify .mcp.json was updated
cat .mcp.json | grep rulebook
```

### Memory Database Corrupted
```bash
# Stats and health check
npx rulebook memory stats

# Cleanup and defragment
npx rulebook memory cleanup

# Clear all memories (destructive)
rm .rulebook/memory/rulebook.db
```

### Task Won't Archive
```bash
# Check what's blocking
npx rulebook task validate <task-id>

# View requirements
npx rulebook task show <task-id>

# Ensure:
# 1. All items in tasks.md are [x] (or marked N/A)
# 2. Docs updated (README, CHANGELOG)
# 3. Tests written + passing
```

### Pre-Commit Hook Failure
```bash
# Run diagnostics
npm run type-check    # See type errors
npm run lint          # See lint issues

# Fix automatically
npm run lint:fix
npm run format

# Then retry commit
git add .
git commit -m "fix(auth): add JWT validation"
```

## Performance Tuning

### Reduce MCP Timeout
For fast machines (default 10s):
```bash
export RULEBOOK_MCP_TIMEOUT_MS=5000
```

### Force SQL.js (No Native Deps)
```bash
export RULEBOOK_MEMORY_BACKEND=sql.js
```

### Speed Up Memory Search
Limit search scope:
```bash
npx rulebook memory search "auth" --type pattern --days 30
```

### Batch Memory Cleanup
Monthly maintenance:
```bash
npx rulebook memory cleanup --older-than 180  # Keep 6 months
```

## CI/CD Integration

### GitHub Actions Workflow
```bash
npx rulebook workflows    # Generate .github/workflows/
```

Generated workflows:
- `.github/workflows/lint.yml` — ESLint
- `.github/workflows/test.yml` — Vitest
- `.github/workflows/build.yml` — TypeScript build
- `.github/workflows/audit.yml` — Dependency audit

### Pre-Commit Framework
```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/hivellm/rulebook
    rev: v5.5.2
    hooks:
      - id: rulebook-format
      - id: rulebook-lint
      - id: rulebook-type-check
```

## Updating Rulebook

```bash
# Check for updates
npm outdated @hivehub/rulebook

# Update to latest
npm install @hivehub/rulebook@latest

# Regenerate rules (preserves AGENTS.override.md)
npx rulebook update

# Verify no breakage
npx rulebook doctor
```

Upgrading is safe:
- `AGENTS.md` regenerated
- `AGENTS.override.md` preserved (survives update)
- `.rulebook/` directory kept intact
- Config merged (new features added)
