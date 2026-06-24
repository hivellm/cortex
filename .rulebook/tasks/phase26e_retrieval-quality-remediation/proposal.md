# Proposal: phase26e_retrieval-quality-remediation

## Why

Source: docs/analysis/cortex/12-live-audit-2026-06-09.md

phase26d live verification of the phase26c retrieval-quality fixes confirmed
three fixes landed (Bug #8 summaries, Bug #9 cache code, Bug #10 ADR status)
but surfaced three residual gaps that the phase26c fixes alone do not close.
These gaps require new work, not just verification:

- **Gap A — Vector scores recovered but below target.** Re-running the audit's
  own measurement (`/v1/query`, repo `cortex`, query `"event classification
  system"`, per-source score) shows the top vector hit improved from `0.130`
  (audit baseline) to `0.238` — a real recovery, but short of the `0.50` bar in
  phase26d §1.3. Root cause is documented in `.rulebook/PLANS.md` (phase0
  `nl-embedding-text-projection`): the NL re-embed was **additive** — new NL
  vectors were inserted but the stale raw-JSON vectors for the same events were
  never purged, so old low-quality vectors still dilute HNSW results. Reaching
  the target needs a purge-then-clean-re-embed of the Vectorizer collections.

- **Gap B — Bundle-cache hit rate is not observable in the split deployment.**
  The phase26c `BundleCache` lives in the `cortex-adapter-claude-code` process;
  `cache_hit_total` / `cache_miss_total` live in that process's
  `PreThinkingMetrics`. The `/v1/health/pre-thinking` endpoint runs in the
  separate `cortex-api` process and is wired with
  `UnwiredPreThinkingHealthSource`, so it always reports `0/0`. The adapter does
  not export the counters over HTTP (its `/v1/health` subsystem `extras` carries
  frame/envelope counters only). Cache correctness is covered by unit tests and
  the backend warm path is fast (~2.5ms measured), but an operator cannot
  observe the live hit rate. Needs the adapter to publish cache counters into an
  observable surface (subsystem `extras` push, or a dedicated adapter endpoint).

- **Gap C — `pre_thinking_p95_ms` dashboard series does not measure pre-thinking.**
  `bucket_p95_duration_per_minute` computes the p95 of `extras["duration_ms"]`
  across *all* recent envelopes, so the series reflects generic envelope
  durations (long tool_calls, builds), not bundle-assembly latency. Live values
  were `2947–43426ms` driven by long tool_calls, while the actual query backend
  warm path is ~2.5ms. The series therefore cannot validate the phase26c cache
  (phase26d §2.2). Needs a dedicated pre-thinking bundle-latency metric distinct
  from the generic envelope-duration series.

## What Changes

### Gap A — purge stale raw-JSON vectors and clean re-embed
- Add a purge path to the embedder / bootstrap re-embed so a re-embed replaces
  (delete-then-insert) rather than adds vectors for an event id.
- Re-embed the `cortex` repo collections and re-measure: top vector hit for the
  audit query must clear `0.50` via the `/v1/query` per-source measurement.
- Backfill the lagging `analyses` index (phase26d found 22% summary coverage)
  before re-embed so its vectors carry NL text.

### Gap B — make adapter bundle-cache counters observable
- Publish `cache_hit_total` / `cache_miss_total` from the adapter into the
  `cortex-adapter` subsystem `extras` (same path frame counters already use), OR
  expose a dedicated adapter health endpoint.
- Verify: two identical pre-thinking queries within the 60s TTL move the live
  counters (`cache_hit_total` increments).

### Gap C — dedicated pre-thinking latency metric
- Record bundle-assembly latency separately from envelope `duration_ms` and
  surface it as its own dashboard series (or rename/repoint
  `pre_thinking_p95_ms` to the real source).
- Verify: series reflects bundle latency and sits below 200ms for repeated
  same-scope/intent queries.

## Impact
- Affected specs: spec 06 (embedder), spec 12 (pre-thinking), spec 26 (dashboard series)
- Affected code: cortex-workers (embedder/re-embed purge), cortex-adapter-claude-code (cache metric export), cortex-api (dashboard series source)
- Breaking change: NO — purge re-embed is idempotent; new metrics are additive
- User benefit: semantic search clears the useful-score bar; operators can see cache hit rate and true pre-thinking latency
