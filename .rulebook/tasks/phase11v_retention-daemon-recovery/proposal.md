# Proposal: phase11v_retention-daemon-recovery

## Why

Six independent gaps make the retention pipeline look broken to anyone
looking at the dashboard, and silently drop two of the four
consolidation-tier sweeps the Cortex retention story relies on.
Discovered while the user was watching `127.0.0.1:5173 → Retention`
and saw every sweep card reading `last run never` despite six of them
having run successfully on `2026-05-04` / `2026-05-05`.

Concretely, the live system on 2026-05-05 03:00 UTC was:

| Sweep                          | Real `cron_jobs` row                              | Dashboard card |
|--------------------------------|---------------------------------------------------|----------------|
| `retention.sweep`              | `last 2026-05-05T03:00:11`, `success`             | `never`        |
| `retention.rollup`             | `last 2026-05-04T04:00:27`, `success`             | `never`        |
| `retention.pii_enforce`        | `last 2026-05-04T05:00:26`, `success`             | `never`        |
| `retention.meili_prune`        | `last 2026-05-04T05:30:26`, `success`             | `never`        |
| `retention.metadata_reap`      | `last 2026-05-04T05:45:26`, `success`             | `4d ago`       |
| `retention.cas_vacuum`         | `last 2026-05-03T04:30:01`, `success`             | `never`        |
| `retention.consolidation_prune`| `failed` — `vectorizer login: HTTP request failed for http://127.0.0.1:17001/auth/login` | `never` |
| `retention.memory_consolidate` | `enabled = 0` (seed-time default was `false`)     | `never`        |
| `retention.turn_digest`        | `next_run_at == last_run_at` (loop)               | `never`        |
| `retention.consolidator_nightly`| absent until container restart re-seeded         | (n/a)          |

Six gaps drive that:

1. **Dashboard `/v1/retention/state` hardcodes `next_run: "never"`.**
   `crates/cortex-api/src/dashboard.rs:3639-3653` iterates over a
   static list of sweep names and stamps `"never"` on every one. The
   test next to it (`dashboard.rs:5067`) freezes the bug as a
   contract: `// Per-sweep next_runs all "never" until phase9k.`
   phase9k landed `retention_daemon` but never wired the dashboard
   handler to the live `cron_jobs` table.
2. **`cortex-ops consolidation-prune` reads the wrong env var.**
   `crates/cortex-cli/src/bin/cortex-ops.rs:4612` reads
   `CORTEX_EMBEDDER_VECTORIZER_URL`, then falls back to the literal
   `http://127.0.0.1:17001`. That literal cannot resolve inside the
   `cortex-api` container — it is the host-side mapped port, not the
   Compose service URL. Every cron-driven prune fails at `auth/login`.
3. **`seed_defaults` is INSERT-only, not UPSERT.** When the default
   for `retention.memory_consolidate` flipped from `enabled: false`
   to `enabled: true` (phase11p §3.2 in
   `crates/cortex-workers/src/retention/scheduler.rs:222`), every
   pre-existing `cron_jobs` row stayed at `enabled = 0`. There is no
   migration step that re-applies a flipped default to already-seeded
   rows.
4. **`next_after()` returns the same instant when the schedule has
   already fired.** `retention.turn_digest` (schedule `0 6 * * 0`)
   shipped `last_run_at == next_run_at == 2026-04-30T20:06:16`. The
   helper appears to be returning the most-recent matching slot
   instead of the next one in the future. The job becomes due every
   tick and fires the LLM-cost-bearing digest at 30 s cadence.
5. **`cortex_consolidations` table never created.** phase11p §3.1
   added the consolidator pipeline but the schema migration that
   carries `cortex_consolidations` (the table the consolidator
   writes into and the prune job sweeps) is absent from the
   `MetadataStore::open` path. Every `consolidation-prune` run that
   gets past env resolution would still find an empty / missing
   table.
6. **`retention_sweeps` table never receives rows.** `cron_jobs`
   shows `last_status = success` but `retention_sweeps` is empty
   (`SELECT count(*) FROM retention_sweeps; → 0`). Every sweep
   binary reports success without writing the canonical
   bookkeeping row, so the dashboard's
   `Bytes reclaimed last 30 d` panel renders zero forever.

## What Changes

- `crates/cortex-api/src/dashboard.rs::retention_state` reads
  `cron_jobs` via the existing `MetadataStore` handle on the
  request-state and projects each row's `next_run_at` /
  `last_run_at` / `last_status` into the response. The hardcoded
  `"never"` list is removed. The frozen-as-contract test is
  rewritten to assert the live-read shape.
- `crates/cortex-cli/src/bin/cortex-ops.rs::consolidation_prune`
  consults `CORTEX_VECTORIZER_URL` (and matching `_USER` /
  `_PASSWORD`) before falling back to the EMBEDDER-prefixed names
  and the loopback literal. `docker-compose.yml` mirrors the values
  onto `CORTEX_EMBEDDER_VECTORIZER_*` for backward compatibility
  with operator scripts that already export the prefixed form.
- `crates/cortex-workers/src/retention/scheduler.rs::seed_defaults`
  becomes idempotent for `enabled` and `schedule`: it INSERTs new
  rows and UPDATEs existing rows whose `(enabled, schedule)`
  diverge from the default, leaving `last_run_at`, `next_run_at`,
  and operator-tuned schedules untouched when they only differ in
  cadence the operator explicitly chose. Drift is recorded via
  tracing.
- `crates/cortex-workers/src/retention/scheduler.rs::next_after`
  always returns a slot strictly greater than `now`. A property
  test (`prop_next_after_strictly_advances`) covers daily, weekly,
  and `Mon 04:30` cadences across a full year of `now` values.
- `crates/cortex-storage/src/metadata.rs` ships the
  `cortex_consolidations` schema as part of the existing
  `apply_phase11_schema` (or a new `apply_phase11p_schema`)
  function called from `MetadataStore::open`. Migration is
  idempotent: `CREATE TABLE IF NOT EXISTS` plus the columns the
  consolidator writes.
- Every `cortex-ops <sweep>` binary writes a `retention_sweeps`
  row via the existing `start_retention_sweep` /
  `finish_retention_sweep` API. A new integration test
  (`tests/retention_sweeps_bookkeeping_it.rs`) drives one of each
  sweep through the in-memory backend and asserts the row count.
- `gui/src/views/Retention.tsx` consumes the new
  `last_run_at` / `last_status` fields and removes the
  `last run never` fallback path now that the API returns honest
  data. The `Run sweep` button keeps its dry-run-only contract.

## Impact

- Affected specs:
  - `docs/specs/19-retention.md` — replace the `next_runs[*] = "never"`
    paragraph with the live-read contract.
  - `docs/specs/02-quantization.md` — already references
    `retention_sweeps`; add a note that every sweep MUST write a
    row, not just the tier_sweep one.
- Affected code:
  - `crates/cortex-api/src/dashboard.rs`
  - `crates/cortex-cli/src/bin/cortex-ops.rs`
  - `crates/cortex-workers/src/retention/scheduler.rs`
  - `crates/cortex-storage/src/metadata.rs`
  - `crates/cortex-workers/src/consolidator/*.rs` (writes to
    `cortex_consolidations`)
  - `gui/src/views/Retention.tsx`
  - `gui/src/views/Retention.test.tsx`
  - `docker-compose.yml`
- Breaking change: NO — operator scripts that already export
  `CORTEX_EMBEDDER_VECTORIZER_*` keep working. The dashboard JSON
  shape gains fields; the GUI is updated in the same PR.
- User benefit: dashboard finally tells the truth; consolidation
  tier prune actually runs; weekly LLM digest stops the 30 s loop
  and matches its declared schedule.
