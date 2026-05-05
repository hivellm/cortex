# Cortex — Self-Indexing & Ingestion Priorities

## What Cortex Should Index About Itself

Since Cortex is designed to be self-referential (it captures interactions across HiveLLM, including interactions with itself), this file specifies redaction concerns and indexing priorities for the Cortex project.

### High-Priority Paths (Index & Keep Fresh)

**Must index:**

| Path | Content | Frequency | Rationale |
|------|---------|-----------|-----------|
| `.rulebook/decisions/` | ADRs, architectural choices | On commit | Decisions made during Cortex implementation are foundational; should be discoverable. |
| `.rulebook/learnings/` | Discoveries, gotchas, drift resolutions | On commit | Every learning (e.g., "Vectorizer SDK 3.0.3 upsert reporting") is a pattern other maintainers need. |
| `.rulebook/knowledge/` | Patterns, anti-patterns | On commit | Cortex itself documents anti-patterns (e.g., "don't bypass SDK without cause"). Should be indexed for self-reference. |
| `docs/analysis/cortex/` | Consolidated health snapshots, roadmap | Weekly or after phase complete | Analysis files are authoritative status reports. |
| `docs/specs/` | Executable spec definitions (SHALL/MUST, scenarios) | On spec update | Specs are source of truth for "what is Cortex supposed to do?" |
| `crates/*/src/**/*.rs` | Source code (Tree-sitter chunks) | On PR | For debugging, decision-tracing ("why was this done this way?"), and code-similarity searches. |
| `CHANGELOG.md`, `AGENTS.md`, `CLAUDE.md` | Project metadata, rules, versioning | On release | Project rules and versioning are context that future sessions need. |

**Medium-priority paths:**

| `crates/cortex-api/src/dashboard.rs` | Dashboard endpoints, aggregator logic | On feature | Useful for understanding how specific dashboard views are fed. |
| `Dockerfile`, `docker-compose.yml` | Deployment, port assignments, env vars | On env change | Operational runbooks depend on accurate port/env mappings. |
| `gui/src/views/` | Dashboard UX, view logic | On UI change | UX decisions (why is Laws read-only now?) are useful context. |
| `.mcp.json` | MCP server tooling, Rulebook setup | On MCP change | Cortex itself uses MCP; changes here affect future sessions. |

**Low-priority paths** (index but with lower freshness SLA):

| `.github/workflows/` | CI/CD definitions | Quarterly | Useful for ops teams; not session-critical. |
| `scripts/` | Utility scripts, test fixtures | On major tool change | Less critical; temporary tools (cypher_load.py, etc.) can be pruned after use. |

### Redaction Rules (High Sensitivity)

Cortex applies PII redaction at ingestion; the following are flagged at higher sensitivity for self-indexing:

| Pattern | Rule | Rationale |
|---------|------|-----------|
| `.env` files | **REDACT** | May contain JWT secrets, Vectorizer/Meili passwords. |
| `docker-compose.yml` env overrides | **PARTIAL** — redact `VECTORIZER_PASSWORD`, `MEILI_MASTER_KEY`, `ANTHROPIC_API_KEY` if present | Credentials in overrides. |
| `.rulebook/secrets/` | **REDACT FULLY** | Reserved for secret storage; should never be indexed. |
| API keys in code comments | **REDACT** | e.g., "use key: sk-..." should become "use key: ..." |
| Test fixture data with mock credentials | **REDACT** | Even mocks; avoid teaching pattern-matching on fake credentials. |
| Email addresses in git log / CHANGELOG | **PRESERVE** (public contacts); **REDACT** if internal only | If CHANGELOG mentions "fixed by @author", preserve the @-mention; redact personal email if present. |

### Frequency & Staleness Tolerance

**Real-time (< 5 minutes):**
- Live interaction capture (adapter events, turns, tool calls).
- Dashboard timeline SSE stream.

**Near-real-time (< 1 hour):**
- Enriched event indexing (after classifier-worker).
- Vector embedding completion.
- Graph node writes (post-Cypher assert).

**Periodic (< 1 day):**
- Decision/learning/knowledge snapshots from `.rulebook/`.
- Analysis file updates.

**Batch (weekly or post-phase):**
- Full-text reindex if Meili has backlog.
- Codebase graph refresh (symbol relations, once phase4c implements them).

### Cortex Ingestion Paths (How to Bootstrap the Cortex Repo)

**Bootstrap command (phase 09):**
```bash
cortex-bootstrap /path/to/Cortex
```

This walks the Cortex repo tree and emits events:
1. **Files** — `.rulebook/decisions/*.md`, `.rulebook/learnings/*.md`, `docs/specs/*.md`, etc. (Tree-sitter chunks → Vectorizer).
2. **Commits** — Git log entries, commit messages (if in scope).
3. **Rulebook entities** — Default-discovery of `.rulebook/tasks/`, memories, decisions.
4. **Archive** — All events stored in Parquet (spec 02).

**Scope** (what cortex-bootstrap discovers):
- Repo type: inferred from presence of `.cortex.toml` or Cortex-specific marker files.
- Entities: tasks, decisions, learnings, memories, analyses (from `.rulebook/`, `docs/`).
- Source: Rust code (via Tree-sitter), Markdown (via pulldown-cmark).

**Exclusions:**
- `target/` (build artifacts).
- `.git/` (history available via commit log; redundant).
- `node_modules/`, `vendor/` (deps; not indexable).
- Secrets (`.env`, `.env.local` — pre-redaction).

### Example Indexing Session (for Cortex maintainers)

```bash
# 1. Bootstrap Cortex itself (populate initial Meili/Vectorizer/Nexus)
docker compose exec cortex-api \
  cortex-bootstrap /workspaces/Cortex \
    --scope cortex \
    --dry-run  # Preview what will be indexed

# 2. Verify coverage
curl http://localhost:17004/indexes  # Meili index list
# Expected: cortex-cortex-misc, cortex-cortex-decisions, cortex-cortex-code, etc.

# 3. Query to validate
curl -X POST http://localhost:17000/v1/query \
  -H 'Content-Type: application/json' \
  -d '{
    "scope": "cortex",
    "query": "ADR graph nodes identity",
    "lanes": ["vector", "keyword", "graph"]
  }'
# Expected: top result = ADR 004, decision details, code references

# 4. Monitor health
curl http://localhost:17000/v1/status
# Check: Meili coverage, Vectorizer collection size, Nexus node count
```

## Cortex as a Canary for HiveLLM Indexing

Cortex should eat its own dog food. When bootstrapping a new HiveLLM repo:

1. **Cortex repo** (self-reference) — verify dashboard queries work correctly.
2. **Rulebook repo** — decisions, learnings, tasks. Verify law/violation federation.
3. **Nexus repo** — code, decisions, issues. Verify graph topology (symbols, functions).
4. **Vectorizer repo** — code, specs, API docs. Verify embedding quality.

Success criteria:
- No indexing errors (all paths reachable).
- Query latency < 100ms (P99).
- Retrieval quality (manual spot-checks of top-5 results).
- No stale data in dashboard (< 5 min old).
- Coverage metrics visible in cortex-api `/v1/dashboard/overview`.

## Future: Federated Indexing Across Hive Repos

When bootstrapping all 17 HiveLLM repos (phase 4b+):

- **Per-repo health dashboard** (dropdown selector in GUI).
- **Cross-repo full-text search** (Meili facet across `cortex-*` indexes).
- **Org-wide decision register** (Meili `cortex-global-decisions` + per-repo rollups).
- **Compliance audit** (global law/violation index for org-wide governance).

Cortex indexing will demonstrate the pattern; other repos follow the same pipeline.
