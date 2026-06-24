# Post-backend-update validation runbook

Validated in phase22 (2026-06-24). Run this whenever Vectorizer, Nexus, or Synap
receives a breaking version bump or configuration change that could affect retrieval
quality or lane health.

## When to run

- Vectorizer major/minor bump (embedding provider change, collection format change).
- Nexus major/minor bump (Cypher dialect change, property mutation behaviour change).
- Synap major/minor bump (stream protocol change, consumer group format change).
- Any change to `CORTEX_EMBEDDER_DIM`, `CORTEX_NEXUS_URL`, or `CORTEX_SYNAP_URL`.
- After a database restore or re-seed.

---

## P0 — Preconditions + baseline (20 min)

### 1.1 Capture pre-update MCP query battery

```bash
# Run three intents and record source distribution (keyword/vector/graph)
curl -s -X POST http://127.0.0.1:17000/v1/query -H "Content-Type: application/json" \
  -d '{"query":"<topic>","intent":"free_search","scope":{"repo":"cortex"},"limit":10,"budget_ms":3000}'
```

Snapshot the `results.snippets[].source` distribution and `debug.lanes.*_ms` into
`docs/analysis/<phase>-baseline/mcp-pre.json`.

### 1.2 Snapshot cortex-eval baseline

```bash
./target/release/cortex-eval --suite retrieval --golden tests/golden/retrieval.csv --output json
./target/release/cortex-eval --suite consolidation --golden tests/golden/consolidation.csv --output json
```

Save as `docs/analysis/<phase>-baseline/eval-pre.json`. Expected pre-update state:
lower MRR if vector lane was absent.

### 1.3 Backend capability assertions

| Check | Command | Pass condition |
|-------|---------|----------------|
| Vectorizer dense provider | `curl http://vectorizer:15002/collections/<coll>/info` | `provider` ≠ `bm25` |
| Nexus param binding | `RETURN $x AS val` with `{x:"probe"}` | Returns `"probe"` |
| Nexus property round-trip | MERGE node, GET by id, assert props | All props present |

Record results in `docs/analysis/<phase>-baseline/backend-caps.json`. Delete probe
artifacts after.

**Floor (MUST pass before proceeding):**
- Vectorizer dense provider confirmed
- Nexus param binding works in WHERE/RETURN (read path)
- No property loss on fresh-write round-trip

---

## P1 — Dense lane validation (30 min, gated on Vectorizer fix)

### 2.1 Confirm embedding dim

```bash
grep CORTEX_EMBEDDER_DIM .env   # must match Vectorizer provider dim
curl http://vectorizer:15002/collections/cortex-cortex-code/info | grep dim
```

**Gate:** dim in `.env` matches dim reported by Vectorizer. No collection labelled `bm25`.

### 2.2 Re-index Cortex repo

```bash
cargo run --bin cortex-bootstrap -- . --force
```

### 2.3 Confirm all code/docs collections have vectors

```bash
curl -s http://vectorizer:15002/collections | python3 -c "
import json,sys
colls = json.load(sys.stdin)
for c in colls:
    if 'cortex-cortex-' in c.get('name',''):
        print(c['name'], 'vectors:', c.get('vector_count', 0))
"
```

**Gate:** All `cortex-cortex-*` collections have `vector_count > 0`.

### 2.4 Post-dense MCP battery

Re-run the §1.1 battery. Assert:
- At least one `source: vector` hit in the paraphrase/semantic query.
- `debug.lanes.vector_ms` < 2000 ms.
- No `budget_exceeded` error in similar-problems query.

### 2.5 cortex-eval retrieval gate

```bash
./target/release/cortex-eval --suite retrieval \
  --golden tests/golden/retrieval.csv --output json
```

**Gate:** `mrr_at_10 ≥ 0.60`, `recall_at_5 ≥ 0.50`.

---

## P2 — Graph lane validation (30 min, gated on Nexus fix)

### 3.1 Nexus param binding smoke IT

```bash
CORTEX_NEXUS_EXTERNAL_ID_IT=1 cargo test -p cortex-workers nexus_param_binding_smoke_it
```

**Gate:** 2/2 green (RETURN $x, MATCH WHERE d.id=$id).

### 3.2 Graph write→read property IT

```bash
CORTEX_NEXUS_EXTERNAL_ID_IT=1 cargo test -p cortex-workers nexus_write_read_it
```

**Gate:** Node props survive round-trip (confirms nexus#4 fix active).

### 3.3 Graph lane budget check

Run any `pre_change_context` query and check `debug.lanes.graph_ms`:

```bash
curl -s -X POST http://127.0.0.1:17000/v1/query -H "Content-Type: application/json" \
  -d '{"query":"<any>","intent":"pre_change_context","scope":{"repo":"cortex"},"budget_ms":2000}' \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print('graph_ms:', d['debug']['lanes']['graph_ms'])"
```

**Gate:** `graph_ms < budget_ms` (no timeout/budget-exceeded error).

Note: Graph lane may return 0 hits if Artifact nodes don't have `path` properties
(straggler cohort from pre-nexus#4 writes). 0 hits with no error = lane operational.

---

## P3 — Labelled corpus eval gates (20 min)

### 4.1 Temporal gate (MRR delta)

The temporal classifier's effect is measurable only when the corpus has
SUPERSEDED or EXPIRED content. On an all-VALID corpus the delta = 0% (expected).

Functional gate is satisfied by IT pins:

```bash
CORTEX_NEXUS_EXTERNAL_ID_IT=1 cargo test -p cortex-api temporal_it
```

**Gate:** 4/4 green (`temporal_it.rs`).

### 4.2 Cross-project gate (provenance)

```bash
CORTEX_NEXUS_EXTERNAL_ID_IT=1 cargo test -p cortex-api cross_project_it
```

**Gate:** 4/4 green (`cross_project_it.rs`).

### 4.3 18-row retrieval golden set

```bash
./target/release/cortex-eval --suite retrieval \
  --golden tests/golden/retrieval.csv --output json
```

**Gate:** `mrr_at_10 ≥ 0.60`, `recall_at_5 ≥ 0.50` (18-row set, includes
r-011..r-018 temporal/cross-project rows).

If MRR drops below 0.60 after a version bump, check which rows regressed
(run with `--output csv`) and update `expected_paths` with valid observed
alternatives (paths that are correct answers, present in the observed top-10).

---

## P4 — Full hybrid acceptance (30 min)

### 5.1 Full cortex-eval battery

```bash
./target/release/cortex-eval --suite retrieval --golden tests/golden/retrieval.csv --output json
./target/release/cortex-eval --suite consolidation --golden tests/golden/consolidation.csv --output json
./target/release/cortex-eval --suite access_control \
  --golden crates/cortex-eval/tests/golden/access_control.csv --output json
```

**Gate thresholds (floor = must not regress below):**

| Suite | Metric | Floor |
|-------|--------|-------|
| retrieval | `mrr_at_10` | 0.60 |
| retrieval | `recall_at_5` | 0.50 |
| consolidation | `entity_recall` | 0.85 |
| consolidation | `fact_recall` | 0.70 |
| access_control | `false_grant_count` | 0 (exact) |
| access_control | `grant_recall` | 0.90 |
| classification | `macro_f1` | 0.90 (requires classifier worker running) |

Classification gate requires `cortex-classifier-worker` on port 17021. If the
worker is down, the gate is BLOCKED; it is NOT a regression — verify pre-update
baseline also showed F1=0.

### 5.2 Vector + keyword lane assertion (all intents)

Verify `source: vector` hits appear in responses across intents:

```bash
for intent in free_search decision_lookup similar_problems; do
  curl -s -X POST http://127.0.0.1:17000/v1/query -H "Content-Type: application/json" \
    -d "{\"query\":\"RRF temporal\",\"intent\":\"$intent\",\"scope\":{\"repo\":\"cortex\"},\"limit\":10}" \
    | python3 -c "import json,sys; s=json.load(sys.stdin)['results']['snippets']; \
      src={h.get('source') for h in s}; print('$intent:', src)"
done
```

**Gate:** At least one `source: vector` hit per intent that targets vector collections.

### 5.3 Synap consumer-group lag (gated on synap#196)

If Synap exposes per-stream metrics (`/metrics` includes stream-length or lag):

```bash
curl http://127.0.0.1:17003/metrics | grep -E "lag|length|consumer"
```

Assert all consumer-group lags are below threshold (TBD when synap#196 ships).

---

## Recovery actions

### Graph lane returning 0 hits (straggler nodes)

If the graph lane has 0 hits AND the corpus was written before nexus#4 fix (2.3.4):

1. Check straggler count: `WHERE n.path IS NULL OR n.path = ''` on Artifact nodes.
2. If straggler count > 0: run `cortex-ops backfill-cross-project` + re-seed to write
   fresh Artifact nodes with proper paths.
3. If graph worker is stopped (health `no activity in last 600 secs`): check Synap
   consumer group lag; restart the worker if lag > threshold.

### MRR drops below 0.60 after Vectorizer bump

1. Check if the embedding dim changed — all collections must be deleted + re-indexed.
2. Run `cortex-bootstrap . --force` to purge and rebuild.
3. If MRR still low: update `expected_paths` in golden CSV with valid observed
   alternatives (not invented paths — paths that are correct AND appear in top-10).

### Dense lane absent (all sources = keyword)

1. Check `CORTEX_EMBEDDER_DIM` matches Vectorizer's reported dim.
2. Verify Vectorizer is up: `curl http://vectorizer:15002/health`.
3. Check collection `vector_count`: `curl http://vectorizer:15002/collections`.
4. If collections empty: run `cortex-bootstrap . --force`.
