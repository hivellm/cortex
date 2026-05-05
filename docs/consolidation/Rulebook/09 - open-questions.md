# Rulebook — Open Questions & Gaps

## Implementation Status

### Fully Implemented (v5.5+)
- Task management (spec-driven, sequential enforcement)
- Persistent memory (hybrid BM25+HNSW search)
- MCP tools (44+, all major categories covered)
- Skills system (enable/disable per project)
- Ralph autonomous loop (multi-iteration, quality gates)
- Workspace support (multi-project coordination)
- Terse mode (v5.4+, output compression)
- Handoff system (context limits, session recovery)
- VSCode extension (dashboard with telemetry)
- Pre-commit/pre-push hooks (language-aware)

### Partial or In-Progress
- **Analysis system** (`rulebook_analysis_create`) — minimal documentation
- **Compression tools** (`rulebook_compress`) — exists but limited testing on large files
- **Telemetry** — opt-in collection; metrics still sparse
- **Security gate details** — varies by language (npm audit fine, cargo audit complete, others sparse)

## Known Limitations

### Memory System
1. **HNSW Index** — no sharding; single index per project
   - Question: Does >1M memories degrade search? Untested.
2. **Embedding computation** — TF-IDF only; no semantic similarity
   - Question: Should we support optional LLM embeddings for better context retrieval?
3. **Privacy redaction** — only `<private>` tags; limited granularity
   - Question: Should we support semantic PII detection + redaction?

### Task System
1. **Cross-project dependencies** — tasks cannot span multiple projects
   - Question: How to model "frontend task depends on API task"?
2. **Blocking** — task cannot be marked "blocked: reason" mid-checklist
   - Question: Should we support explicit blocking status separate from deferral?
3. **Subtasks** — tasks.md is flat; no nesting
   - Question: How to break large tasks into subtasks without creating separate tasks?

### Ralph Loop
1. **Context estimation** — heuristic-based; varies by model/token-counter
   - Question: Should we use actual token counting (tiktoken) for accuracy?
2. **Parallel story execution** — limited to 3 stories at once
   - Question: What's the optimal parallelism for cost vs. quality?
3. **Learning extraction** — manual annotation, not automated
   - Question: Can we auto-extract patterns from specs + code diffs?

### Workspace
1. **Cross-workspace queries** — only single workspace supported
   - Question: Should Rulebook support federation (e.g., querying linked Rulebook instances)?
2. **Project discovery** — auto-detects from manifest files (pnpm.yaml, turbo.json)
   - Question: What about flat monorepos without automation metadata?

### Rules System
1. **Language-specific rules** — 28 languages supported; coverage varies
   - Question: Are all language-specific specs complete or templated?
2. **Framework-specific rules** — 17 frameworks; some are stubs
   - Question: Which frameworks need more detailed specs?
3. **Tool-specific configs** — 23 AI tools; some are CLI-only
   - Question: How to handle tools without CLI (e.g., web-only IDEs)?

## Unanswered Design Questions

### 1. Should Rulebook Manage Dependencies?

**Current**: No integration with npm, cargo, pip, etc.

**Question**: Should `rulebook check-deps` also suggest updates + auto-update?
- **Pro**: Unified dependency management
- **Con**: Language-specific tooling (npm, cargo, pip) is mature; don't reinvent

### 2. Should Ralph Have Budget Controls?

**Current**: `--max-iterations` cap; no cost estimation.

**Question**: Should Ralph estimate cost (API calls × model pricing) before starting?
- **Pro**: User sees upfront cost
- **Con**: Requires model pricing config; diverges for Claude 3.5 vs. Opus

### 3. Should Memory Support Structured Metadata?

**Current**: Free-form tags (`["pattern", "bug"]`); no schema.

**Question**: Should memory entries have typed fields (e.g., `{pattern_name, applicability, languages[]}`)?
- **Pro**: Better indexing + filtering
- **Con**: Rigid schema conflicts with ad-hoc memory saves

### 4. Should Rulebook Integrate with External VCS?

**Current**: Git-aware (hooks, commit messages); no integration with GitHub API.

**Question**: Should Rulebook auto-create issues from tasks + sync status?
- **Pro**: GitHub becomes single source of truth
- **Con**: GitHub API dependency; not all projects use GitHub

### 5. How Should Rulebook Handle Secrets?

**Current**: `<private>` tag for manual redaction; no integration with secret managers.

**Question**: Should Rulebook detect + redact API keys, credentials, tokens?
- **Pro**: Prevent accidental leaks to memory
- **Con**: PII detection is imperfect; could hide legitimate examples

## Gaps vs. Cortex

### 1. No Node Creation from Tasks

**Gap**: Rulebook tasks don't auto-populate Cortex nodes.

**Current workaround**: Manual consolidation step (read tasks → create nodes).

**Question**: Should `rulebook_task_archive` auto-create Cortex nodes?

### 2. No Bidirectional Sync

**Gap**: Cortex node updates don't flow back to Rulebook.

**Current workaround**: Manual updates in `.rulebook/` after editing Cortex.

**Question**: Should Cortex write back to `.rulebook/decisions/` and `.rulebook/knowledge/`?

### 3. No Event Streaming

**Gap**: Rulebook publishes no webhooks; Cortex polls changes.

**Current workaround**: Cron job checking `.rulebook/tasks/` + memory timestamps.

**Question**: Should Rulebook emit events (task.created, memory.saved, etc.) to Cortex?

### 4. No Query Optimization

**Gap**: Memory search is local-only; no aggregation across projects.

**Current workaround**: Cortex must call `rulebook_workspace_search` per project.

**Question**: Should workspace support cross-project memory queries?

## Performance Unknowns

1. **Memory search latency** — claimed <10ms for 100k memories; untested at 1M+
2. **Ralph iteration time** — varies wildly by task complexity + model; no benchmarks
3. **MCP tool overhead** — measured at ~50ms per tool call; includes startup
4. **Hook execution time** — pre-commit hooks add ~2–5s per commit; impact on large repos?

## Language Coverage Questions

### Incomplete Specs
- **Dart**: Template only, no detailed spec
- **Ada, SAS, Lisp, Objective-C**: Minimal support
- **R, Zig, Erlang**: Templates generated but not validated
- **Solidity** (blockchain): Supported but no smart contract specifics

### Framework Gaps
- **Spring Boot**: Partial (no deployment specs)
- **Symfony**: Template only
- **Electron**: Minimal (no OS-specific rules)
- **React Native, Flutter**: Limited mobile-specific guidance

## Testing Unknowns

1. **Coverage claims** — 95%+ reported; actual coverage by feature?
2. **Integration tests** — are all 44 MCP tools integration-tested?
3. **Cross-platform** — tested on macOS, Linux; Windows (WSL) edge cases?
4. **Memory DB** — crash recovery, corruption scenarios untested?

## Recommended Consolidation Priorities

For Cortex ingestion:

1. **High Priority**:
   - Sync task metadata + archival triggers
   - Ingest memory index (patterns, learnings, decisions)
   - Monitor quality gate pass/fail
   - Implement task → node creation

2. **Medium Priority**:
   - Bidirectional sync (Cortex → .rulebook/)
   - Event streaming (webhook or polling)
   - Cross-project query optimization

3. **Low Priority** (future):
   - External secret manager integration
   - Semantic memory embeddings option
   - Cross-workspace federation
