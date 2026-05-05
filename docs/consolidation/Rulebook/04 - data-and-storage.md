# Rulebook Data & Storage

## .rulebook/ Directory Layout

```
.rulebook/
├── rulebook.json               # Configuration (version, features, terse mode)
├── STATE.md                    # Live task/health status (machine-written)
├── PLANS.md                    # Session scratchpad (human-written)
├── specs/
│   ├── RULEBOOK.md            # Task management spec
│   ├── AGENT_AUTOMATION.md    # Agent workflow spec
│   └── <language>/
│       ├── spec.md            # Language-specific requirements
│       └── templates/         # Code templates
├── tasks/
│   ├── <phase>_<task-id>/
│   │   ├── proposal.md        # Why and What Changes
│   │   ├── tasks.md           # Checklist items only
│   │   ├── design.md          # Optional: technical design
│   │   └── specs/
│   │       ├── <module>/spec.md # Module-specific spec
│   │       └── ...
│   └── ...
├── archive/
│   ├── 2026-05-04-<task-id>/  # Dated completed task
│   └── ...
├── memory/
│   ├── rulebook.db            # SQLite database
│   │   └── Tables:
│   │       ├── memories        # Key-value storage
│   │       ├── embeddings      # HNSW vectors (256-dim)
│   │       ├── metadata        # Tags, types, timestamps
│   │       └── timeline        # Chronological index
│   └── temp.wasm              # sql.js WASM fallback
├── knowledge/
│   ├── <id>.md                # Pattern or anti-pattern
│   └── index.json             # Searchable index
├── learnings/
│   ├── <id>.md                # Implementation insight
│   └── index.json             # Searchable index
├── decisions/
│   ├── <id>.md                # ADR (status: proposed/accepted/superseded)
│   └── index.json             # Searchable index
├── handoff/
│   ├── _pending.md            # Pending handoff context
│   └── <session-id>.md        # Historical handoffs
└── hooks/                     # Git hooks + SessionStart/Stop hooks
    ├── pre-commit            # Type-check, lint, format
    ├── pre-push              # Tests, coverage, security
    ├── SessionStart           # Load rules, activate terse mode
    ├── UserPromptSubmit       # Inject terse attention anchor
    └── Stop                   # Handoff trigger at context limits
```

## Memory Database Schema

### Tables

#### `memories`
```
id TEXT PRIMARY KEY
content TEXT           # Full context
tags TEXT[]           # JSON array: ["pattern", "bug", "discovery"]
type TEXT             # "memory", "pattern", "anti-pattern", "learning", "decision"
created_at TIMESTAMP
updated_at TIMESTAMP
source TEXT           # "memory_save", "task_archive", "manual"
project_id TEXT       # For workspace isolation
```

#### `embeddings`
```
memory_id TEXT PRIMARY KEY  # FK: memories.id
vector FLOAT32[256]         # TF-IDF normalized
```

#### `timeline_index`
```
id TEXT                     # memory_id
created_at TIMESTAMP
type TEXT
```

#### `metadata`
```
memory_id TEXT
key TEXT
value TEXT
```

### Search Algorithm

1. **Query Parsing** — tokenize and normalize input
2. **BM25 Ranking** — keyword relevance (IDF-weighted)
3. **Vector Embedding** — TF-IDF similarity (256-dim)
4. **Hybrid Fusion** — Reciprocal Rank Fusion (RRF)
   - Combine top-k BM25 + top-k vector results
   - RRF score = 1/(k + rank)
5. **Rerank** — sort by fused score, apply recency decay

### Persistence

- **Default**: better-sqlite3 (native binding, best performance)
- **Fallback**: sql.js (pure JavaScript + WASM, no native deps)
- **Privacy**: Auto-redact `<private>` tags during save/search
- **Cleanup**: Auto-evict entries older than 180 days (configurable)

## Task Directory Structure

Every task in `.rulebook/tasks/<phase>_<task-id>/`:

```
proposal.md
  Why section (≥20 characters required)
  What Changes section
  Success Criteria

tasks.md
  - [ ] Phase § Item 1
  - [x] Phase § Item 2 (completed)
  - [ ] Phase § Item 3 (deferred: reason)

design.md (optional)
  Architecture decisions
  Trade-offs
  Diagrams

specs/
  core/spec.md
  api/spec.md
  ...
```

### Spec Format

```markdown
## ADDED Requirements
### Requirement: Name
The system SHALL/MUST <do something>.

#### Scenario: Name
Given <context>
When <action>
Then <outcome>

## MODIFIED Requirements
...

## REMOVED Requirements
...

## RENAMED Requirements
...
```

## Configuration Files

### `.rulebook/rulebook.json`
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
    "defaultMode": "brief",
    "intensityLevels": {
      "off": 0,
      "brief": 1,
      "terse": 2,
      "ultra": 3
    }
  },
  "memory": {
    "backend": "better-sqlite3",
    "evictionDays": 180,
    "autoRedact": true
  },
  "workspace": {
    "enabled": false,
    "projects": []
  }
}
```

## Handoff System

When context reaches threshold (default 75%, forced at 90%):

1. **SessionStart hook** — checks context usage
2. **Trigger** — if `>75%`, suggest `/handoff`; if `>90%`, mandate it
3. **Write** — `rulebook_memory_session_end` writes `.rulebook/handoff/_pending.md`:
   ```markdown
   # Session Handoff
   
   **Active Task**: phase11l_nexus-external-ids-migration
   **Progress**: §2.3 complete, §3 in progress
   **Files Touched**: src/api/cortex.ts, crates/cortex-storage/src/archive.rs
   **Decisions**: Used HNSW over FAISS for memory search
   **Next Steps**: Complete §3, run full test suite
   **Resume**: `rulebook task show phase11l_nexus-external-ids-migration`
   ```
4. **Restore** — SessionStart hook in next session loads `_pending.md` into memory

## Archival

When task completes:
1. All items in `tasks.md` are `[x]` (or marked N/A)
2. Mandatory tail met: docs updated, tests written, tests pass
3. `rulebook task archive <task-id>` validates and moves to `.rulebook/archive/<date>-<task-id>/`
4. Auto-captures learnings and patterns to memory
5. Task becomes read-only in archive

## Multi-Project Isolation

In workspaces:
- Each project has isolated `.rulebook/` directory
- Memory entries tagged with `project_id`
- `rulebook workspace search` filters by project
- Tasks scoped to single project (cannot span across)
