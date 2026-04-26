# 12 — Pre-thinking injection

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 10, 11

## Goal

Turn the raw bundle returned by the query API (spec 11) into a **compact, well-shaped block of context** that the Claude Code adapter (spec 10) drops into the model's system prompt *before* the model plans its response. The goal is not "maximize context" — it is "give the model exactly the 3–5 things it needs to avoid repeating past mistakes, honor active laws, and reference the right decisions." This spec owns the heuristics, formatting, and budget.

## Scope

**In:**
- Adapter-side module (`cortex-adapters/claude-code/src/pre_thinking.rs`) that wraps `cortex-api /v1/query`.
- Scope-derivation heuristics from the user prompt + `cwd` + recent files.
- Bundle formatter (deterministic Markdown; no model-generated prose).
- Byte-budget enforcement (≤32 KB per `adapter.pre_thinking.max_bundle_kb`).
- Per-section caps (snippets N, decisions N, laws N) with fairness.
- Debug tracing: every bundle assembled carries a `query_id` so we can audit retrieval quality later.

**Out:**
- Query lanes (spec 11).
- Hook wiring (spec 10).
- Evaluation harness / offline scoring (Phase 2 retrieval-quality pass).
- Non-Claude-Code adapters (spec 17 copies this module with adapter-specific tweaks).

## Inputs / Outputs

### Input (from the adapter)

```rust
pub struct PreThinkingInput<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub user_prompt: &'a str,
    pub cwd: &'a Path,
    pub recent_files: &'a [RecentFile],   // from git status, TTL-cached 10 s (spec 10)
    pub budget: PreThinkingBudget,
}

pub struct RecentFile {
    pub path: PathBuf,
    pub status: FileStatus,                // modified | staged | untracked
    pub age_seconds: u64,
}

pub struct PreThinkingBudget {
    pub bundle_bytes: u32,                 // default 32 * 1024
    pub time_ms: u32,                      // default 600 (hook-budget bound)
}
```

### Output (string returned as `additionalContext`)

A deterministic Markdown block. Example:

```markdown
<!-- cortex: pre_change_context · query_id=01HY… · budget=32KB -->

## Active laws in this scope
- **LAW-012** (notable) — HNSW recall benchmarks must run before merge.
- **LAW-007** (critical) — Never pass `--no-verify` to `git commit` without explicit authorization.

## Recent decisions you should know about
- **DEC-0042 (accepted, 2026-03-05)** — Raise HNSW ef_search default to 128.
  Rationale: recall@10 held above 0.92 up to 2M vectors in benchmarks.
- **DEC-0031 (superseded-by DEC-0042)** — Keep ef_search=64 for low-latency profile.

## Similar past turns
1. 2026-02-11 — Claude Sonnet refactored `hnsw_search` to accept `ef` per-call.
2. 2025-12-03 — Gemini benchmarked ef_search=128 and concluded it was safe up to 2M.

## Relevant snippets (3)
1. `Vectorizer/src/index/hnsw/mod.rs:hnsw_search` — current implementation with configurable `ef`.
2. `Vectorizer/docs/perf/hnsw.md#ef_search-tuning` — section on recall/latency tradeoffs.
3. `Vectorizer/benches/hnsw_recall.rs:bench_ef` — benchmark that drives LAW-012.

<!-- end cortex -->
```

Sections are always in the same order: **laws → decisions → similar turns → snippets → (optional) graph neighbors**. Sections with zero entries are omitted entirely (no empty headers). The trailing comment makes it easy to strip / diff in logs.

## Design

### Pipeline

```
user_prompt + cwd + recent_files
        │
        ▼
  scope_derive()  ──────────▶   QueryRequest (spec 11)
        │                                │
        │                                ▼
        │                         cortex-api /v1/query
        │                                │
        │                                ▼
        └──────▶ bundle_format() ◀─── QueryResponse
                        │
                        ▼
           clip_to_budget() + audit()
                        │
                        ▼
                   additionalContext
```

### `scope_derive`

Maps `(user_prompt, cwd, recent_files)` → `scope`:

1. **repo** — basename of the nearest ancestor containing `.git/` (or the `cortex.toml` `cortex.id` override).
2. **files** — union of:
   - `recent_files` (age < 5 min)
   - files mentioned verbatim in the user prompt (shell-glob-like regex; bounded to 16 candidates)
3. **topics** — none by default. Leave topic filtering to fusion-side reranking.
4. **since** — `None` in v1; we want all relevant history.

If `repo` can't be resolved, the adapter issues the query with repo-less scope and accepts coarser results.

### `intent` selection

Rule-based (no ML). The `user_prompt` is classified by cheap keyword match:

| Signal in prompt                                              | Intent                   |
|---------------------------------------------------------------|--------------------------|
| contains "refactor", "modify", "rewrite", "change", tool edits | `pre_change_context`     |
| contains "why", "who decided", "should we"                    | `decision_lookup`        |
| contains "stuck", "keep failing", "doesn't work"              | `similar_problems`       |
| contains "can I", "is it allowed", "blocked"                  | `law_check`              |
| otherwise                                                      | `pre_change_context`     |

`pre_change_context` is the safe default — it pulls the broadest mix.

### Budget-aware section caps

Per section, soft caps:

| Section          | Max entries (default) | Max bytes per entry |
|------------------|-----------------------|---------------------|
| Laws             | 10                    | 256                 |
| Decisions        | 5                     | 512                 |
| Similar turns    | 5                     | 256                 |
| Snippets         | 5                     | 1 024               |
| Graph neighbors  | 0 (off by default)    | 256                 |

After formatting, the total is measured. If it exceeds `budget.bundle_bytes`:

1. Drop graph neighbors (if present).
2. Trim snippets to their `why` + first 3 lines of `text`.
3. Halve the snippets count.
4. Halve the similar-turns count.
5. Truncate decision bodies to 160 chars.
6. As a last resort, drop snippets entirely.

Never drop **laws** — active laws are load-bearing; better to drop everything else than to ship a prompt that silently skips a blocking rule.

### Deterministic formatting

- No templating engine — pure Rust string concatenation with fixed section order.
- Markdown is stable across runs (same input → byte-identical output).
- `query_id` is injected in the leading comment for auditability (spec 11 audit stream correlation).

### Error handling (fail-open)

| Failure                       | Response                                              |
|-------------------------------|-------------------------------------------------------|
| `scope_derive` fails          | Issue query with `scope = {}`; still useful            |
| `cortex-api` timeout (>600 ms) | Return empty string; session unaffected (spec 10 rule) |
| `cortex-api` 5xx              | Return empty string; log + metric                      |
| 0 results in the response      | Return empty string (not an empty header block)        |
| Any formatter panic            | Return empty string; never crash the daemon            |

### Observability

```
cortex.prethink.calls.total        counter, labels: intent
cortex.prethink.bundle.bytes       histogram
cortex.prethink.sections.count     histogram, labels: section
cortex.prethink.truncation.applied counter, labels: step (1..6)
cortex.prethink.latency_ms         histogram
cortex.prethink.empty_bundle       counter  // 0-result responses
cortex.prethink.timeouts           counter
```

Every call emits a span with `query_id`, `intent`, `scope_hash`, `bundle_bytes`, `sections_included`.

## Acceptance criteria

- [ ] Given a user prompt "refactor hnsw_search to take ef per call" in the Vectorizer repo, `scope_derive` produces `repo=Vectorizer, files=[src/index/hnsw/mod.rs]`, `intent=pre_change_context`.
- [ ] Given a 3-KB response with 2 laws, 3 decisions, 4 snippets, the formatter emits a bundle with all four sections in fixed order and size < 4 KB.
- [ ] Budget enforcement: artificial response of 80 KB is clipped to ≤32 KB; clip steps execute in the documented order; laws section is preserved.
- [ ] Empty-result response → empty string returned; counter `prethink.empty_bundle` increments.
- [ ] Timeout: forced 800 ms API latency (budget=600 ms) → empty string, no partial bundle, counter `prethink.timeouts` increments.
- [ ] Intent selection: a prompt containing "why did we pick 128?" maps to `decision_lookup`.
- [ ] Deterministic output: identical inputs produce byte-identical bundles across 1 000 runs.
- [ ] `query_id` is present in the leading comment and matches the Cortex audit stream entry.
- [ ] Truncation: a snippet `text` of 5 KB is trimmed to 1 024 bytes in the bundle; original length is preserved in debug logs.
- [ ] When `recent_files` is empty, the query still issues (repo-scope only) and returns non-empty results on the bootstrap corpus.
- [ ] Laws are **never** dropped: a bundle request with 20 active laws keeps 10 (the cap) and drops snippets/decisions/turns first to fit the budget.
- [ ] Unit test: formatter round-trip for a fixture response is stable byte-for-byte.

## Decisions

1. **Rules, not a model, to pick intent.** A small rule table is fast, debuggable, and predictable. We graduate to a model only if offline eval shows >5% precision gap.
2. **Laws are load-bearing — never trim.** Other sections can shrink or disappear; laws stay.
3. **Fixed section order, no prose.** The model relies on structural cues more than stylistic ones; a stable, scannable layout is better than natural-language "Here are the things I found…".
4. **Empty-result → empty bundle.** Injecting "No relevant context found." would train models to ignore the block. Silence is more honest.
5. **No model-generated summaries at read time.** Summaries already exist (classifier, spec 05). Re-summarizing here would add latency and non-determinism.
6. **Per-section caps, not a global ranker.** Simpler and avoids pathological bundles dominated by one section.
7. **`query_id` in a comment.** Survives any Markdown pass-through and lets us correlate bundle quality with retrieval audit later.

## Open questions

1. **Intent routing via an MCP tool.** Should the model pick its own intent (a dropdown) instead of the adapter guessing? Leaning no (UX cost, latency) but revisit if intent mismatch shows up as the dominant failure mode.
2. **Adaptive budgets.** A 32-KB cap is a hunch. Once we have eval data, tune per intent (e.g., `similar_problems` wants more snippets).

## References

- Architecture §5.3 (query → context bundle), §8 (end-to-end example step 2).
- Spec 01 — Event schema (nothing direct; used via the query response).
- Spec 10 — Claude Code adapter (embeds this module; owns the hook budget).
- Spec 11 — Query API (response schema is this spec's input).
- Spec 13 — Laws DSL (laws content comes from here).
- Spec 14 — Governance engine (trust score could reweight caps later; not in v1).
- Spec 17 — Additional adapters (will copy this module with small surface differences).
