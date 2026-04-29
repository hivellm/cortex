# cortex-retention

> Spec: [`docs/specs/19-retention.md`](../../docs/specs/19-retention.md),
> [`docs/specs/02-storage-layout.md`](../../docs/specs/02-storage-layout.md)

Phase 9 retention engine. Implements the periodic sweeps that keep
Cortex's storage tiered, compact, and policy-compliant. The CLI surface
lives in `cortex-cli`'s `cortex-ops` binary so operators run everything
through one entry point; this crate is the library that does the work.

## Sweeps

| Module           | Spec           | What it does                                                                                                |
|------------------|----------------|-------------------------------------------------------------------------------------------------------------|
| _root_ (`lib.rs`)| 02 §quantization | Vectorizer tier transitions: FP32 → PQ at 30 days, PQ → Binary at 365 days. Idempotent re-encode + upsert. |
| `parquet_rollup` | 19 §archive    | Rolls aged NDJSON+zstd archives into Parquet partitions following the canonical layout.                    |
| `cas_vacuum`     | 19 §cas        | Reclaims unreferenced blobs from the SQLite-backed CAS, capped per run.                                    |
| `pii_enforce`    | 19 §pii        | Applies redaction policies to retained events past their grace window.                                     |
| `turn_digest`    | 19 §digests    | Summarises long LLM turns into compact digests so cold tiers stay queryable.                               |
| `meili_prune`    | 19 §fulltext   | Drops Meilisearch documents whose source events have aged out of the hot index.                            |

## Library shape

```rust
use cortex_retention::{
    SweepPlan, run_sweep,
    cas_vacuum, meili_prune, parquet_rollup, pii_enforce, turn_digest,
};

let plan = SweepPlan::default(); // now=Utc::now(), batch limits, hot-collection allow-list
let report = run_sweep(&plan, &vectorizer_ops, &meta).await?;
println!("{} re-encoded, {} skipped", report.reencoded, report.skipped);
```

Each sweep takes a small `*Ops` trait so production wires the live SDK
clients (`vectorizer-sdk`, Meilisearch HTTP, SQLite handles) and tests
swap in deterministic in-memory fakes. Every sweep is **idempotent**:
re-running on the same window after success is a no-op.

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Operator workflow

Sweeps are scheduled by `cortex-ops` (see `cortex-cli`):

```bash
cortex-ops retention sweep --tier fp32-to-pq
cortex-ops retention sweep --tier pq-to-binary
cortex-ops retention vacuum-cas
cortex-ops retention rollup-parquet
cortex-ops retention prune-meili
```

Every run writes a `retention_sweeps` row through
`cortex_storage::MetadataStore` so the dashboard and `doctor` can audit
when each tier last moved.

## Testing

```bash
cargo test -p cortex-retention
```

Round-trip tests cover: tier-cutoff arithmetic, idempotency under repeat
runs, CAS reference counting, Parquet partition naming, PII grace
windows, and digest determinism. No live backends required.

## Stability

Pre-1.0 — adding a new sweep module is a minor change; changing the
tier-age thresholds or partition layout is a breaking change because it
shifts what data lives where.
