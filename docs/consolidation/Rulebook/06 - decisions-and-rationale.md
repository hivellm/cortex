# Rulebook Decisions & Rationale

## Task Workflow Philosophy

### Why Mandatory Task Creation Before Implementation

**Problem**: AI agents implement features without tracking, causing:
- Lost context after session ends
- No record of why changes were made
- Impossible to resume work mid-task
- No validation of completion criteria

**Solution**: Mandatory task registration via `rulebook task create`:
- All features tracked in `.rulebook/tasks/`
- Progress visible and measurable
- Easy resumption after handoff
- Clear acceptance criteria in specs

**Enforcement**: `enforce-mcp-for-tasks` hook blocks manual `mkdir` in `.rulebook/tasks/`; tasks must be created via MCP tool.

### Why Phase-Numbered Sequences Exist

**Problem**: Cherry-picking tasks out of order creates dependencies:
- Earlier tasks may have implicit prerequisites
- Later tasks may depend on earlier changes
- Reordering breaks parallel progress tracking

**Solution**: Phase-numbered task IDs (`phase11l_nexus-external-ids-migration`) enforce strict sequence:
- Each task declares its phase
- Lowest-numbered phase runs first
- No skipping ahead to "easier" items
- Dependencies documented in proposal.md

**Enforcement**: Agent automation rule `LAW-CORTEX-001` (in Cortex's `AGENTS.override.md`) mandates sequence.

### Why Mandatory Tail Items Exist (Docs + Tests + Verify)

**Problem**: Incomplete implementations shipped to production:
- Features without documentation
- Code without tests
- Untested changes committed

**Solution**: `rulebook task archive` refuses to close tasks without:
1. **Docs** — README/CHANGELOG updated for public APIs
2. **Tests** — meaningful tests written + passing
3. **Verify** — manual verification or E2E proof

**Enforcement**: Archive validation rejects incomplete tasks.

## Memory System Design

### Why Hybrid BM25 + HNSW Instead of LLM Embeddings

**Problem**: Pure LLM embeddings require API calls, adding:
- Latency (100-200ms per query)
- Cost (API charges)
- Privacy risk (data sent to 3rd party)
- Dependency on external service

**Solution**: Local-only hybrid search:
- **BM25** — keyword relevance (IDF-weighted, proven recall)
- **HNSW** — dense vector similarity (256-dim TF-IDF, no LLM)
- **RRF** — Reciprocal Rank Fusion combines both

**Benefits**:
- Instant queries (local DB, no API)
- Zero privacy concerns (offline storage)
- Deterministic results (TF-IDF, not neural randomness)
- Works offline (no internet required)

### Why 256-Dimensional TF-IDF Vectors

**Trade-off**: Dimensionality vs. index size vs. recall
- **Too low** (<64): Loses semantic nuance
- **256**: Good balance; captures context terms well
- **Too high** (>512): Increases DB size, minimal recall gain

**TF-IDF over LLM embeddings**:
- Deterministic (same query always ranks same)
- No model bias
- Works for all languages equally
- Cheap to compute locally

### Why HNSW Index Structure

**Comparison**:
- Flat L2 — O(n) search, simple but slow
- IVF (Product Quantization) — fast but lossy
- **HNSW** — logarithmic search, exact results, memory-efficient

HNSW chosen because:
- ~100k memories searched in <10ms
- Exact results (no approximation loss)
- Handles updates without full reindex
- Works in WASM (sql.js fallback)

## OpenSpec Compatibility

### Why Adopt OpenSpec Format

**Source**: OpenSpec defined spec-driven development with:
- Phase-prefixed task IDs
- Proposal + tasks.md + specs/ structure
- SHALL/MUST keywords (testable requirements)
- Given/When/Then scenarios (executable specs)

**Rulebook adoption**:
- 100% OpenSpec-compatible format
- Auto-migration from OpenSpec to Rulebook
- Extends with archival + memory integration
- Task sequence enforcement (Rulebook addition)

### Why Deprecate OpenSpec in Rulebook

**Decision**: Rulebook ships with full task management; OpenSpec tooling no longer maintained.

**Rationale**:
- Rulebook MCP tools are feature-superset of OpenSpec CLI
- Integrated memory + task management (OpenSpec lacked memory)
- Better error messages, validation
- Active maintenance in HiveLLM ecosystem

**Migration path**:
- Existing OpenSpec tasks auto-converted on `rulebook init`
- Old task IDs preserved
- Specs remain in same format

## Ralph Autonomous Loop Design

### Why Multi-Iteration with Fresh Context

**Problem**: Single-shot AI task solving fails on complex tasks:
- Model runs out of context mid-solution
- Cannot refine based on test failures
- No learning across attempts

**Solution**: Ralph loop (v5.0+):
```
Iteration 1: Read PRD → Implement → Quality gates → Learn
Iteration 2: Refine based on learnings → Quality gates → Learn
...
Until all stories pass
```

**Benefits**:
- Fresh context per iteration (terse mode compresses state)
- Learning extraction (patterns saved to memory)
- Graceful pause/resume
- Parallel story execution (independent features in parallel)

### Why 5 Quality Gates (Not 1 or 10)

**Gates**: Type-check → Lint → Tests → Coverage → Security

**Why these 5**:
1. **Type-check** — catches structural errors (fastest)
2. **Lint** — style + code smell issues
3. **Tests** — behavioral correctness
4. **Coverage** — ≥95% required (ensures testing)
5. **Security** — dependency audit + static analysis

**Rationale**: Each gate catches different error class. Ordering is cheapest-first (type-check in milliseconds, tests in seconds).

## Structural Enforcement Philosophy

### Why PreToolUse Hooks Over Post-Commit Checks

**Alternative**: Let edits land, then validate in CI/CD
- Slower feedback (commit → push → CI run → failure)
- Bad commits pollute history
- Agent frustrated by repeated rejects

**Chosen**: PreToolUse hooks block at tool level
- Instant feedback (<1ms)
- Never reaches disk if forbidden
- Clear error message shows why blocked
- Agent learns pattern on first try

**Three hooks**:
1. `enforce-no-deferred` — blocks TODO/FIXME in tasks.md
2. `enforce-no-shortcuts` — blocks stubs/TODOs in source
3. `enforce-mcp-for-tasks` — blocks manual mkdir in .rulebook/

## Terse Mode Design (v5.4+)

### Why Structured Compression Instead of "Be Brief"

**Problem**: Generic "answer concisely" instruction:
- Steers toward structured output (headings, code blocks)
- Inflates tokens vs. no instruction (71% larger in eval)
- Inconsistent compression across models

**Solution**: Terse Mode (SessionStart + UserPromptSubmit hooks):
1. **SessionStart** — write `.rulebook/.terse-mode` with intensity
2. **SKILL.md injection** — filtered to remove low-priority advice based on intensity
3. **UserPromptSubmit anchor** — ~45-token attention anchor per message

**Intensity levels**:
- `off` — no compression
- `brief` — short agent prompts
- `terse` — strict context limits
- `ultra` — maximum compression for CI/automation

**Measured lift** (vs. baseline):
- Terse: 58% token reduction (34–77% per-prompt range)
- Honest delta: 58% average improvement

## Decision Record System

### Why ADR Format with Lifecycle

**Problem**: Decisions made and forgotten; rationale lost to time.

**Solution**: Architecture Decision Records (ADRs) with states:
- **proposed** — candidate decision under discussion
- **accepted** — decision agreed
- **superseded** — newer decision replaces this
- **deprecated** — decision obsolete but kept for reference

**Never deleted** — supersede instead (maintains history).

Examples:
- "Use HNSW over FAISS for memory search" — ADR-001 (accepted)
- "Hybrid BM25+vector over pure LLM embeddings" — ADR-002 (accepted)
- "Phase-numbered task sequences" — ADR-003 (accepted)

Stored in `.rulebook/decisions/` with searchable index.
