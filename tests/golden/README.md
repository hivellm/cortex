# Cortex golden-set fixtures (phase14c)

This directory holds the curated CSVs the `cortex-eval` harness runs each suite against. Three CSVs, one per suite:

| File | Suite | Min rows | Floor metric(s) |
|---|---|---|---|
| `retrieval.csv` | retrieval | 100 | MRR@10 ≥ 0.60, recall@5 ≥ 0.50 |
| `consolidation.csv` | consolidation | 50 | entity-recall ≥ 0.85, fact-recall ≥ 0.70 |
| `classification.csv` | classification | 200 | macro-F1 ≥ 0.90 |

The CSVs are deliberately hand-curated from real Cortex traffic so they reflect the agent's actual usage patterns rather than synthetic queries.

## Curation cadence

- **Initial seed**: this commit ships a starter set extracted from live archive traffic + canonical phase decisions. Treat the starter as the regression floor — CI gate fails any PR that drops > 5% below it (phase14c §4).
- **Quarterly refresh**: the operator extends each CSV with 10–20 rows per quarter capturing new feature areas and new failure modes spotted in dashboard reviews.
- **Per-incident additions**: any post-mortem that surfaces a retrieval / consolidation / classification regression MUST add a golden row reproducing the regression — that row stays in the set forever.

## File contracts

### `retrieval.csv`

```csv
id,query,repo,expected_event_ids
r-001,"how does tier sweep work",cortex,01HXEVT001;01HXEVT002
```

- `id` — stable opaque label (e.g. `r-001`). Used in per-row diagnostics.
- `query` — verbatim text the harness sends to `POST /v1/query`.
- `repo` — repo scope, or empty for cross-repo.
- `expected_event_ids` — `;`-delimited list of event ids the row expects in the top-10 results.

### `consolidation.csv`

```csv
id,session_id,expected_entities,expected_facts
c-001,01HXSESS00,"HNSW;ef_search;recall@10","ef_search=128;recall>=0.92"
```

- `id` — stable opaque label.
- `session_id` — ULID of the session whose consolidation the row pins.
- `expected_entities` — `;`-delimited proper-noun phrases the consolidation MUST mention.
- `expected_facts` — `;`-delimited claim-shaped phrases (assertions about behavior, decisions, configurations).

### `classification.csv`

```csv
id,envelope_json,expected_kind
cl-001,"{""tool"":""claude-code"",""payload"":{}}",Turn
```

- `id` — stable opaque label.
- `envelope_json` — single-line JSON envelope the classifier receives.
- `expected_kind` — canonical `Kind` label.

## CSV editing tips

- Escape inner double quotes with `""` per RFC 4180. The eval harness parses with `csv` crate's strict mode + `trim=All`.
- Keep `expected_*` lists short (≤ 5 entries per row) so a single regression doesn't tank the suite score.
- Add a one-line comment row (`# ...`) at the top documenting the row source — `cortex-eval` skips lines starting with `#` during load.

## Running locally

```bash
cargo build --release -p cortex-eval
./target/release/cortex-eval --suite retrieval --golden tests/golden/retrieval.csv --output md
./target/release/cortex-eval --suite consolidation --golden tests/golden/consolidation.csv
./target/release/cortex-eval --suite classification --golden tests/golden/classification.csv
```

Set `--baseline path/to/baseline.json` to compare against a prior report; exit 2 = regression > threshold (default 0.05).
