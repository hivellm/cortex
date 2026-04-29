# Relevance harness — labeled query set

This directory holds the fixture consumed by the
[`cortex-relevance-eval`](../../crates/cortex-relevance-eval) binary
(phase6e / closes [F-008](../../docs/analysis/relevance/01-findings.md)).

`queries.toml` is the source of truth: ≥10 queries per intent across
the five intents (`pre_change_context`, `decision_lookup`,
`similar_problems`, `law_check`, `explain`). Stable ids (`rel-NNN`)
let CI diffs surface only the rows that actually changed.

The harness scores each query as `recall@10` (did any expected id
appear in the top-10 fused snippets?) and `MRR` (`1 / rank` of the
first match, `0` if absent). Per-intent + global aggregates land in
`target/relevance/<git-sha>.json`. The CI gate fails the run on a
≥2pp regression on global recall or MRR vs the `main` baseline.

## Schema

```toml
version = 1                  # bump if the schema breaks

[[queries]]
id = "rel-001"               # stable, never reused
intent = "pre_change_context"
query = "the meili fan-out worker offset"
expected_doc_ids = [
  "crates/cortex-fulltext/src/routing.rs",
  "evt:cortex-Cortex-misc:01ABC...",
]
notes = "Operator audit prompt — worker leaves stale offsets in Synap."
[queries.scope]
repo = "Cortex"
files = []                    # optional
topics = []                   # optional
since = "2026-01-01T00:00:00Z" # optional
```

Required fields: `id`, `intent`, `query`, `expected_doc_ids` (≥1).
`scope.repo` is strongly recommended — without it the strategies layer
routes to `cortex-unknown-*` and every lane returns zero hits
(F-003), which silently floors the recall numbers.

## How `expected_doc_ids` matches a snippet

The harness derives a canonical doc id from each returned snippet:

```
{repo|"_"}|{path|"_"}|{content_hash|"_"}
```

…and matches each `expected_doc_ids` entry against (in order):

1. The canonical composite id, exact equality.
2. Any individual snippet field — `repo`, `path`, `symbol`,
   `content_hash`, `collection` — exact equality.
3. **Substring match** against `path` or `symbol` only. This last
   resort lets you write `"crates/cortex-api/src/strategies.rs"` once
   and have it match every chunk hash variant.

Pick the form that maps cleanly to the kind of evidence the query
expects:

| Evidence type            | Recommended id form                          |
| ------------------------ | -------------------------------------------- |
| A specific code file     | `crates/.../foo.rs` (substring on `path`)    |
| A specific decision file | `.rulebook/decisions/2026-...md`             |
| A specific event chunk   | `sha256:...` (exact `content_hash`)          |
| A whole module           | partial path like `crates/cortex-graph`      |
| A topic / theme          | a tag like `redaction` (substring fallback)  |

## Curation process

1. **Pick a real prompt.** Walk the audit log
   (`cortex.events.query_audit`) or recent session memory; copy the
   exact wording the operator typed. Synthetic prompts ("how does X
   work?") are fine for the `explain` bucket but should be a
   minority — most entries should be questions someone has actually
   asked the daemon.
2. **Choose the intent** the pre-thinking router *should* land on.
   When you're unsure, run the prompt through
   `cortex-pre-thinking::intent_select` once and copy the chosen
   intent — this lets the harness double as a router-stability test.
3. **Find the load-bearing answer.** Open the result the operator
   would have wanted. Its repo-relative path or content hash is the
   `expected_doc_ids` entry. For overlapping answers (eg. the
   strategies module *and* the spec doc), list both — the harness
   counts a hit when *any* expected id appears.
4. **Run the harness locally** to make sure your entry passes:
   ```bash
   cargo run -p cortex-relevance-eval -- \
     --query-set tests/relevance/queries.toml \
     --api-url http://127.0.0.1:17000
   ```
5. **Never reuse an id.** Stable ids are how CI diffs work — when an
   entry is removed, leave the id retired rather than recycling it.

## Determinism

The harness produces deterministic results for a given index state
when no sampling is involved. It captures `api_version` from
`/v1/status` in the report header so investigators can trace a
regression to the underlying daemon version. If a backend (Vectorizer
/ Meili / Nexus) is down at boot, the harness records the affected
intent buckets in `omitted_intents` instead of failing the run.

## Coverage budget

Aim for ~50 entries today, ≥10 per intent. The harness is fast (one
HTTP round-trip per query, parallelism is wasteful given the fan-out
inside `cortex-api`), so growing the set linearly is safe — but each
new entry adds noise to the regression gate, so prefer rotating in
high-signal queries over keeping marginal ones.
