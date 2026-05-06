# 03 — Retrieval relevance audit

> **User pain**: "the data so far doesn't result in anything actually
> relevant."
>
> **Verdict**: relevance dies before ranking. **Tier 1 (CRITICAL)**:
> queries without `scope.repo` fall back to `cortex-unknown-*` (zero
> hits). **Tier 2 (HIGH)**: 2 of 3 repos have empty Meili indices while
> Vectorizer holds 10K+ embeddings. **Tier 3 (HIGH)**: graph
> topologically flat (only `IN_REPO` + `REMEMBERS`). RRF, fusion,
> projections, query rewriting — several already closed in prior
> phases; what remains is **scope routing + indexing coverage + graph
> mapper**.

---

## Symptoms

1. Pre-thinking bundle returns `Relevant snippets (1)` with empty/null
   content even when Meili/Vectorizer are populated.
2. Results don't scale with indexed corpus size (< 5 hits regardless).
3. Cross-repo queries fail silently — no `scope.repo` header lands on
   `cortex-unknown-{family}` (zero hits).
4. Keyword lane degenerate on 2 of 3 repos (Meili=0 docs vs
   Vector=10K+).
5. Graph lane effectively disabled — only 2 edge types
   (`IN_REPO`, `REMEMBERS`) of ~12 spec'd.
6. Intent routing misses common phrasings ("explain X", "how does Y
   work" fall through to `pre_change_context` instead of dedicated
   intent).

---

## Current pipeline

```
User Prompt (MCP/Dashboard/GUI)
        ↓
[QueryRequest] {scope: Option<repo>, query, limit=20, k=50}
        ↓
[Scope Resolution]
   ├─ Explicit (header x-cortex-repo)
   ├─ CWD-derived (header x-cortex-cwd)
   └─ FALLBACK: cortex-unknown-* (returns 0 hits) ⚠️
        ↓
[Intent → Strategy]  (5 intents + 1 default fallback)
        ↓
[Fan-out: 3 lanes in parallel]
   ├─ Vector (Vectorizer)  → per-repo collections {code,docs,decisions,
   │                          consolidation,topic-cards}
   ├─ Keyword (Meili)      → per-repo indices {code,docs,decisions,
   │                          consolidations,laws,topic-cards}
   └─ Graph (Nexus)        → templates {edge_artifact_touched_neighbours,
                              decision_supersedes_chain, ...}
        ↓
[RRF Fusion: α=0.7, k=60]
   fused = Σ [ 0.7 * (1/(60+rank)) + 0.3 * normalized_score ]
   ├─ Dedup on (repo, path, symbol)
   ├─ Truncate → limit=20
   └─ Tie-break: recency → severity → doc_id
        ↓
[Overlays: post-fusion enrichment]
   ├─ Decisions      (requires extras["decision_id"])
   ├─ Laws           (requires extras["law_id"])
   ├─ Graph neighbors
   └─ Similar turns  (requires extras["turn_id"])
        ↓
[Budget clipper] (32 KB → 4 KB target)
   1. drop graph neighbours
   2. slim snippets (why + 3 lines)
   3. halve snippets
   4. halve similar turns
   5. truncate decisions
   6. drop snippets (last resort)
        ↓
[Response] {snippets, decisions, violations, similar_turns, graph_neighbors}
```

---

## Findings

### Finding 1 — Scope routing broken for MCP / direct HTTP callers
- **File**: `crates/cortex-api/src/strategies.rs:25-34`
- **Problem**: `repo_scoped()` falls back to `UNKNOWN_REPO_SLUG` when
  `req.scope.repo` is `None`. Pre-thinking derives via CWD walk; direct
  HTTP callers (MCP, dashboard, GUI) do NOT. Headers `x-cortex-repo` /
  `x-cortex-cwd` are optional.
- **Evidence**: `Service::query()` in `service.rs` accepts raw
  `QueryRequest`; no scope fallback. `resolve_scope()` only runs with
  headers.
- **Severity**: **CRITICAL** — any query without explicit repo hits
  `cortex-unknown-{family}` → zero hits across all lanes.
- **Status**: open; spec-04j calls for HTTP 422 but not wired.

### Finding 2 — Meili indices undersupplied on 2 of 3 repos
- **File**: `crates/cortex-workers/src/fulltext/routing.rs` +
  `crates/cortex-api/src/meili_loader.rs`
- **Problem**: at boot meili_loader finds
  `cortex-cortex-turns=589`, `cortex-rulebook-*=0`,
  `cortex-vectorizer-*=0`.
- **Evidence**: ingest lag or routing misconfig; lane returns empty;
  orchestrator dedup + truncate yields empty result.
- **Severity**: **HIGH** — fundamental indexing gap.

### Finding 3 — Graph lane: 2 edge types vs ~12 spec'd
- **File**: `crates/cortex-api/src/nexus_graph_lane.rs:61-100` +
  `docs/analysis/cortex/03-data-quality.md:64-98`
- **Problem**: `cortex-graph::mapper` doesn't consume the chunker's
  `symbol` field. Nexus has only `IN_REPO` (10,245) and `REMEMBERS`
  (30) edges. Spec defines `CALLS`, `IMPORTS`, `DEFINES`, `RETURNS`,
  `VIOLATES`, `SUPERSEDES`, etc.
- **Evidence**: `edge_artifact_touched_neighbours` template
  topologically flat; nearly zero hits.
- **Severity**: **HIGH** — graph lane returns sparse/uninformative.

### Finding 4 — Vector lane payload deserialization (already closed in phase11d)
- **File**: `crates/cortex-api/src/vectorizer_lane.rs:19-34`
- **Problem (was)**: SDK `SearchResult { content, metadata }` didn't
  match server `{id, score, payload}`. Payload (path, body, kind)
  silently dropped → all hits had empty text.
- **Status**: ✅ **CLOSED** (phase11d — direct reqwest POST bypasses
  SDK).

### Finding 5 — Keyword projection chain (already closed in phase6g)
- **File**: `crates/cortex-api/src/meili_lane.rs:175-216`
- **Problem (was)**: artifacts had `summary=""`, `title=path`,
  `body=code`. Chain `summary > title > body` stopped at `title`.
- **Status**: ✅ **CLOSED** (phase6g — kind-aware projection).

### Finding 6 — Lane projection contract extras (already closed in phase6b)
- **File**: `crates/cortex-api/src/lanes.rs:44-53` +
  `crates/cortex-api/src/orchestrator.rs:341-368`
- **Problem (was)**: live lanes didn't stamp
  `extras["decision_id"]`/`turn_id`/`law_id`. Overlays always `[]` in
  prod.
- **Status**: ✅ **CLOSED** (phase6b — both lanes project contract
  keys; regression test `lane_contract.rs`).

### Finding 7 — Query rewriting (already closed in phase6f)
- **File**: `crates/cortex-api/src/query_rewrite.rs`
- **Problem (was)**: PassthroughRewriter default; embedding via the
  full question ("why is meili broken — should we rewrite?") instead of
  load-bearing noun phrases.
- **Status**: ✅ **CLOSED** (phase6f — wired in, audit envelope stamps
  rewritten form). Operators tune via `CORTEX_QUERY_REWRITER`.

### Finding 8 — Intent selector (already closed in phase6d)
- **File**: `crates/cortex-pre-thinking/src/intent_select.rs:21-112`
- **Problem (was)**: 5-6 keywords per intent; missing "how does X
  work", "what is X", "explain X", etc. Fell through to
  `pre_change_context`.
- **Status**: ✅ **CLOSED** (phase6d — `Intent::Explain` + extended
  tables).

### Finding 9 — Pre-thinking hardcodes `limit=20`, `k=50`
- **File**: `crates/cortex-pre-thinking/src/pipeline.rs:121-140`
- **Problem**: no per-intent tuning. `explain` intent requests 20
  snippets and the clipper truncates to 4-8. `k=50` for vector is
  generous; top-10 already covers what's relevant.
- **Severity**: **LOW** — clipper compensates; CPU waste only.
- **Status**: ACK; no fix needed.

### Finding 10 — RRF α=0.7 favors rank over native score
- **File**: `crates/cortex-api/src/fusion.rs:31-34`
- **Problem**: formula `0.7 * (1/(60+rank)) + 0.3 * normalized_score`.
  Positional component dominates; graph lane with `score=0.0` lets
  weak hits compete on rank alone.
- **Evidence**: regression
  `weak_graph_hit_does_not_outrank_dense_vector_top3()`
  (fusion.rs:562) pins the guardrail.
- **Severity**: **MEDIUM** — tunable via `CORTEX_RRF_ALPHA`.

### Finding 11 — Portuguese analyzer not configured in Meili
- **File**: `crates/cortex-workers/src/fulltext/settings/settings.v1.json`
- **Problem**: user is Brazilian (queries in pt-BR: "consolidações",
  "por que"). Settings v1 uses default English. No pt-BR stopwords,
  no stemmer.
- **Severity**: **MEDIUM** — pt-BR queries may miss inflected forms.
- **Status**: open.

### Finding 12 — No schema validation on `/v1/query`
- **File**: `crates/cortex-api/src/service.rs`
- **Problem**: caller can pass `scope.repo = ""`; `repo_scoped()`
  treats it as `None` and falls back to unknown. No early rejection.
  Empty query, negative limit, zero budget accepted.
- **Severity**: **LOW** — no crash; spec-04j calls for 422.

---

## Where relevance dies (root causes ranked)

### Tier 1 — Scope routing (CRITICAL)
1. **MCP / direct HTTP callers don't pass `scope.repo` → fallthrough
   to `cortex-unknown-*` → zero hits.** Wire `x-cortex-repo` into
   service layer; reject 422 when unresolved.

### Tier 2 — Indexing coverage (HIGH)
2. **Meili indices Rulebook + Vectorizer empty while Cortex has 589
   docs.** Ingest lag or routing misconfig. Check fulltext-worker
   queue, confirm routing rules, rebuild.
3. **Graph lane flat (only IN_REPO + REMEMBERS).** cortex-graph mapper
   doesn't consume `symbol` field from chunker. phase4c (already
   tracked).

### Tier 3 — Tuning / Polish (MEDIUM/LOW)
4. **RRF α=0.7 — single weak graph hit can outrank vector top-10.**
   Mitigated by phase6c (score-aware blend); tunable.
5. **Portuguese analyzer absent.** Settings v7 + pt-BR analyzer.
6. **No validation for empty `scope.repo`.** spec-04j: 422; wire
   resolver before orchestrator.

---

## Phased rework plan

### Phase 1 — Scope routing fix (1-2 days)
- `Service::query()` calls `resolve_scope()` with headers.
- HTTP 422 when scope unresolved + `CORTEX_ALLOW_UNKNOWN_SCOPE` unset.
- Audit envelope stamps `scope_resolution ∈ {explicit, header, cwd, rejected_legacy}`.
- MCP `cortex_query` adds `x-cortex-repo` from adapter cwd.
- Dashboard search bar adds `x-cortex-cwd`.
- **Verify**:
  - `cortex_query query="consolidation" scope.repo="Rulebook"` → snippets > 0
  - `curl -X POST /v1/query -d '{"query": "test"}'` → HTTP 422
  - Header override routes to correct repo.

### Phase 2 — Indexing coverage audit (1-2 days)
- Boot meili_loader with `debug=true` logging seeded counts per repo.
- Verify cortex-rulebook-* / cortex-vectorizer-* receive events.
- Spot-check 10 docs per family.
- Manual re-index if any repo is zero-doc.
- **Verify**:
  - `cortex meili-search --index cortex-rulebook-code --query "schema"` returns >0
  - `docker logs cortex-fulltext-worker | grep upserted` shows activity
  - Per-repo golden-set: keyword + vector both > 0.

### Phase 3 — Verify lane projection contracts (1 day)
- Run `cargo test -p cortex-api --test http :: overlay`.
- Spot-check live Vectorizer payload + Meili docs for `decision_id`,
  `turn_id`.
- Audit envelope `counts.decisions > 0` on decision-space queries.
- **Verify**: regression suite green; `audit.counts.decisions > 0`.

### Phase 4 — Intent routing coverage (1 day)
- Unit tests: 11 "explain" phrasings → `Intent::Explain`.
- Extended pre_change_context / decision_lookup / similar_problems
  tables.
- Spot-check 5 live queries.
- **Verify**: `cargo test -p cortex-pre-thinking ::intent_select`
  passes; spot-check `audit.intent="explain"`.

### Phase 5 — Golden-set framework (2-3 days) **MANDATORY before further tuning**
- CSV `docs/evals/queries.csv`: 30-50 queries with labeled relevant
  docs (repo, path, kind, reason).
- Crate `cortex-eval` binary: MRR@10, recall@5, snippet BLEU.
- CI gate: blocks commits that degrade MRR > 5%.
- Pre-rework baseline + post-rework delta.
- **Verify**:
  - `cortex-eval --golden-set docs/evals/queries.csv --output baseline.json`
  - Post-rework: `mrr_at_10 ≥ 0.60`, `recall_at_5 ≥ 0.50`,
    `snippet_bleu ≥ 0.35`.

### Phase 6 — Polish (1 day)
- Bump Meili settings v7: pt-BR analyzer + stopwords.
- Rebuild indexes.
- API validation: reject empty scope.repo, negative limit, zero
  budget.
- **Verify**:
  - Query `"consolidações de estilo"` returns stemmed variants.
  - `limit: -5` → 422; `scope.repo: ""` → 422.

---

## Key files (absolute paths)

**Scope resolution**:
- `E:\HiveLLM\Cortex\crates\cortex-api\src\service.rs`
- `E:\HiveLLM\Cortex\crates\cortex-api\src\strategies.rs`

**Lane projection contracts**:
- `E:\HiveLLM\Cortex\crates\cortex-api\src\meili_lane.rs`
- `E:\HiveLLM\Cortex\crates\cortex-api\src\vectorizer_lane.rs`
- `E:\HiveLLM\Cortex\crates\cortex-api\src\lane_contract.rs`

**Indexing / fulltext**:
- `E:\HiveLLM\Cortex\crates\cortex-workers\src\fulltext\routing.rs`
- `E:\HiveLLM\Cortex\crates\cortex-api\src\meili_loader.rs`
- `E:\HiveLLM\Cortex\crates\cortex-workers\src\fulltext\settings\settings.v1.json`

**Intent routing**:
- `E:\HiveLLM\Cortex\crates\cortex-pre-thinking\src\intent_select.rs`
- `E:\HiveLLM\Cortex\crates\cortex-api\src\strategies.rs`

**Eval framework** (to create):
- `E:\HiveLLM\Cortex\docs\evals\queries.csv`
- `E:\HiveLLM\Cortex\crates\cortex-eval\` (new crate)

**Pre-thinking pipeline**:
- `E:\HiveLLM\Cortex\crates\cortex-pre-thinking\src\pipeline.rs`
- `E:\HiveLLM\Cortex\crates\cortex-pre-thinking\src\budget.rs`

**Audit / observability**:
- `E:\HiveLLM\Cortex\crates\cortex-api\src\audit.rs`
