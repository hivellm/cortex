# VectorizerOps trait abstraction makes tier-sweep tests deterministic

**Category**: storage
**Tags**: phase9a, retention, trait, testing, cortex

## Description

Tier-transition sweepers have a fundamental coupling to the Vectorizer SDK that makes them hard to test. Solution: define a `VectorizerOps` trait with the minimal four-method surface the sweep needs (`list_older_than`, `dest_has`, `reencode_and_upsert`, `delete_from`) and ship a `MemoryVectorizerOps` test double that round-trips records via an in-memory `BTreeMap<String, Vec<RecordRef>>`. Production wiring then provides one trait impl backed by the live SDK; tests hit the same `run_sweep(plan, ops)` entry with the in-memory ops. Every spec scenario (FP32→PQ at 31 d, idempotent re-run, dry-run, ceiling-trip drop rate) becomes a 20-line unit test.

## Example

#[async_trait]
pub trait VectorizerOps: Send + Sync {
    async fn list_older_than(&self, c: &str, cutoff: DateTime<Utc>, n: u32) -> Result<Vec<RecordRef>, SweepError>;
    async fn dest_has(&self, dest: &str, id: &str) -> Result<bool, SweepError>;
    async fn reencode_and_upsert(&self, dest: &str, tier: Tier, r: &RecordRef) -> Result<u32, SweepError>;
    async fn delete_from(&self, src: &str, id: &str) -> Result<(), SweepError>;
}
// MemoryVectorizerOps implements the trait via a Mutex<BTreeMap<...>>;
// `inject_upsert_error_once` lets tests drive the dropped-record path
// deterministically.

## When to Use

Any sweep / migration / re-encoding flow that walks an external store. Pattern generalises to Meili archival pruner (phase9f), CAS vacuum (phase9c), parquet rollup compactor (phase9b) — same trait shape, different impls.

## When NOT to Use

When the trait surface needs more than ~5 methods or when the production impl needs back-pressure/streaming the trait doesn't model. At that point the trait drowns the production code in adapter glue and a real integration harness pays for itself.
