# 02 — Memory cleanup / forget / retention audit

> **User pain**: "memory cleanup has to be brute force."
>
> **Verdict**: delete paths exist for Vectorizer/Meili/Nexus, but the
> **Parquet archive is only purged via `/v1/admin/forget` per-event**
> (no cron), the **CAS vacuum fails silently** behind a 50% safeguard,
> and **digest purges are opt-in** (`--purge-originals` off by default).
> Operator falls back to `rm -rf` on the filesystem.

---

## Symptoms

- Disk grows indefinitely even after retention cron runs.
- Operator must delete Parquet files manually to free space.
- `/v1/admin/forget` works per-event but no tooling exists for date
  ranges.
- Sweeps report success but old data persists.
- Live-file partial zstd frames break rewrite during peak ingestion.

---

## Inventory of stores and their delete paths

| Store | Write path | Delete path | Status |
|-------|-----------|-------------|--------|
| **Vectorizer** | `cortex.{turn,tool_call,code_chunk}.{fp32,pq}` + `cortex.cold.binary` | `VectorizerClient::delete_vectors(collection, [event_id])` | ✅ Wired to `/v1/admin/forget` |
| **Meili** | `cortex_{turns,tool_calls,consolidations,decisions,...}` | `delete_documents(index, [event_id])` | ✅ Wired to `/v1/admin/forget` |
| **Nexus** (graph) | nodes with `event_id` property | `MATCH (n {event_id: $id}) DETACH DELETE n` | ✅ Wired to `/v1/admin/forget` |
| **Parquet archive** | `events/year=/month=/day=/hour=/*.parquet` | rewrite excluding `event_id` | ⚠️ Tolerates live-file partial frames (commit `766a74b`) BUT **only called by `/v1/admin/forget`, not by cron** |
| **SQLite metadata** (`cron_jobs`, `retention_sweeps`, `cortex_consolidations`) | `metadata.sqlite` | rows reaped by `metadata_reap` cron | ✅ Cron-driven |
| **CAS** (content-addressable) | blob hashes referenced by code chunks | `delete_blobs([hash])` via `cas_vacuum` | ⚠️ Safeguard ≥50% **silently returns Ok(0)** instead of error |
| **Queue/WAL state** | retention job rows, cron logs | persist in `retention_sweeps` + `cron_jobs` | ❌ No delete path; rows accumulate forever |
| **Audit envelope queue** | bootstrap envelopes, lifecycle events | not enumerated in code | ❓ Retention contract unknown |

---

## Findings

### P0 — Critical architectural gaps

#### Finding 1 — Archive never deleted by normal sweeps
- **File**: `crates/cortex-api/src/admin_forget.rs:164-181`
  (`LiveArchivePurger::drop_event`) +
  `crates/cortex-workers/src/pruner/purge.rs:122-168`
- **Problem**: the **only** delete path for the Parquet archive lives
  in `/v1/admin/forget`. The cron pruner (`pruner/engine.rs`) demotes
  vectors in Vectorizer and strips fields from Meili, but **never
  rewrites the Parquet partition**. Old events stay in archive forever.
- **Evidence**: `purge::forget` calls `archive.drop_event()` but is only
  reachable via the HTTP endpoint. No `cortex-ops` subcommand for
  standalone archive purging.
- **Severity**: **P0** — violates the WORM/retention contract.

#### Finding 2 — Pruner expired-tier doesn't cascade to all backends
- **File**: `crates/cortex-workers/src/pruner/engine.rs` + `mod.rs:160-165`
- **Problem**: when a consolidation reaches `PruneTier::Expired` (>365
  days):
  - Vectorizer: vectors hard-purged ✅
  - Meili: rows deleted ✅
  - **Nexus: nodes not deleted** ❌
  - **Parquet archive: rows not removed** ❌
- **Evidence**: `plan_demotion(Expired)` returns `DemotionAction { from:
  Cold, to: Expired, vector_ids: [] }` — `vector_ids` empty because
  cold shards aren't queried. Purge sink only reachable via admin
  endpoint. **No code path** `plan_demotion(Expired) → purge::forget()`.
- **Severity**: **P0** — graph becomes a zombie with references to
  expired events.

#### Finding 3 — CAS vacuum fails silently
- **File**: `crates/cortex-workers/src/retention/cas_vacuum.rs:80-130`
- **Problem**: safeguard refuses to delete when >50% of blobs would be
  dropped. Returns `Ok(report)` with `blobs_dropped=0` instead of
  `Err`. Operator never sees that cleanup failed.
- **Evidence**: line 120 — `if force_enabled ... && (would_drop * 2) >
  report.total_blobs`. Line 141 — `store.delete_blobs(&hashes)?` never
  called. Comment line 91: "safeguard tripped: ... pass --force to
  override".
- **Severity**: **P0** — cleanup is opt-in and invisible to operator.

### P1 — Silent failures and missing cascades

#### Finding 4 — Tool-call digest purge is opt-in
- **File**: `crates/cortex-workers/src/retention/tool_call_digest.rs` +
  spec-19
- **Problem**: cron default is dry-run; `--purge-originals` must be
  passed explicitly. Operator forgets, data is never deleted.
- **Evidence**: spec-19 line 45-48 — "Default mode is preview: no
  classifier call, no deletes".
- **Severity**: **P1** — tool-call envelopes accumulate silently.

#### Finding 5 — Metadata reap and CAS vacuum don't talk
- **File**: `crates/cortex-workers/src/retention/metadata_reap.rs` +
  `cas_vacuum.rs`
- **Problem**: when `metadata_reap` deletes rows from
  `cortex_consolidations`, it doesn't signal `cas_vacuum` to enqueue
  orphan blobs. Vacuum has to rescan the entire corpus on each run
  (O(corpus) work).
- **Evidence**: spec-19 lists both as separate jobs; no bidirectional
  binding.
- **Severity**: **P1** — orphans accumulate, vacuum needs `--force`.

#### Finding 6 — Live-file tolerance only in admin_forget
- **File**: `crates/cortex-api/src/admin_forget.rs:218-322` (commit
  `766a74b`)
- **Problem**: `is_live_partial_frame()` was added ONLY in admin_forget,
  not in `turn_digest`/`tool_call_digest` purge paths nor `cortex-ops
  retention-sweep` archive handling. When digest purgers run at
  06:00/06:30 (peak ingestion), they hit the active current-hour
  partition, the error aborts the rewrite, the counter stays at 0, and
  the operator never sees the failure.
- **Evidence**: commit `766a74b` is surgical — "fix(admin_forget):
  tolerate live-file partial zstd frames". Other purgers have no
  equivalent.
- **Severity**: **P1** — silent race condition with ingestion.

### P2 — Data leaks and visibility gaps

#### Finding 7 — Meili loader doesn't react to consolidation deletes
- **File**: `crates/cortex-api/src/meili_loader.rs:200-202`
- **Problem**: loader seeds consolidations into the
  `MemoryKeywordLane` for the dashboard, but no subscriber to
  `meili_sink::purge()`. When `/v1/admin/forget` deletes a consolidation
  from Meili, the lane carries a stale hit until the next refresh.
- **Severity**: **P2** — cosmetic but confuses operator.

#### Finding 8 — Retention daemon doesn't surface errors to dashboard
- **File**: `crates/cortex-api/src/retention_daemon.rs:151-207`
- **Problem**: ticks every 30s, errors logged locally (line 186-191) but
  not persisted to a shared alert queue. Dashboard
  `/v1/retention/state` reads `cron_jobs.last_status` but transient
  error doesn't escalate to UI. Comment line 203-206: "surfacing it to
  the rest of cortex-api lands in a follow-up (phase10l)".
- **Evidence**: line 207 — `let _ = scheduler.drain_warnings().await;`
  warnings drained without persistence.
- **Severity**: **P2** — delayed observability (operator finds out the
  next day).

#### Finding 9 — Archive lacks date-range retention policy
- **File**: `crates/cortex-storage/src/archive.rs` (inferred)
- **Problem**: spec promises archive delete/downsample, but no
  `archive_prune` cron exists nor an API for "drop all events before
  2026-01-01". Operator falls back to `rm -rf events/`.
- **Severity**: **P2** — missing operational tooling.

#### Finding 10 — No coordinated shutdown on partial failure
- **File**: `crates/cortex-workers/src/pruner/engine.rs`
- **Problem**: pruner with partial failure (Vectorizer ok, Meili fails)
  returns `Ok` with incomplete state. Operator doesn't know to retry.
- **Severity**: **P2** — cross-store consistency risk.

---

## Why brute force is needed (root causes ranked)

1. **No automatic archive deletion** — only path is `/v1/admin/forget`
   per-event. Operator falls back to `rm -rf events/` when archive
   grows.
2. **CAS safeguard without force-override in cron** — vacuum fails
   silently ≥50%; operator runs manually with `--force`.
3. **Digest purgers are opt-in** — tool-call digest cron default doesn't
   include `--purge-originals`; envelopes never deleted.
4. **Live-file race condition** — sweeps at 06:00 hit the active
   current-hour partition; rewrite fails, purge counter 0, no alert.
5. **Silent failures** — daemon logs locally, dashboard discovers hours
   later via the table.

---

## Rework plan

### Phase 1 — Automatic archive pipeline (gate: `rm -rf` unnecessary)
- Build `cortex-ops retention-archive-purge --before <RFC3339>
  [--dry-run] [--json]`.
- Wire it into `retention/scheduler.rs:237-280` as cron `0 4 * * *`
  (default disabled).
- Explicit safeguard: error if >50% of archive would drop (not silent).
- Integrate with consolidation pruner expired-tier (callback trait).
- **Verify**: `cortex-ops retention-archive-purge --dry-run` lists N
  partitions; second run with same `--before` reports 0 (idempotent);
  dashboard shows `last_run_at`.

### Phase 2 — Tool-call digest purge default-on (gate: automatic cleanup)
- Change cron default in `scheduler.rs:237-280`:
  - **From**: `cortex-ops tool-call-digest --budget-cents 500`
  - **To**: `cortex-ops tool-call-digest --budget-cents 500 --purge-originals`
- Env `CORTEX_DIGEST_DRY_RUN=true` for dev opt-out.
- **Verify**: cron log shows `records_purged: N` every Sunday 06:30.

### Phase 3 — Shared live-file tolerance (gate: no ERRORs in peak hours)
- Extract `is_live_partial_frame()` + `rewrite_partition_safe()` into
  `crates/cortex-storage/src/archive_purge.rs`.
- Apply to `turn_digest` + `tool_call_digest` + `cortex-ops
  retention-sweep`.
- Log DEBUG (not ERROR) on deferral; retry on next rotation.
- **Verify**: digest purger during peak hour → "deferred" logs, no
  errors; next run completes.

### Phase 4 — CAS vacuum 3-tier safeguard (gate: never silent)
- New logic in `cas_vacuum.rs:80-130`:
  - `would_drop < 5%` → delete normally
  - `5% ≤ would_drop < 50%` → delete + log WARN
  - `would_drop ≥ 50%` → `Err(SafeguardTripped)` (not Ok(0))
- Env `CORTEX_CAS_VACUUM_FORCE=true` bypasses the 50% limit.
- Separate cron `cas-vacuum --force` (default disabled).
- **Verify**: normal run with >50% returns exit 1; forced cron when
  enabled overrides.

### Phase 5 — Consolidation pruner cascade (gate: expired = gone everywhere)
- Wire `pruner/engine.rs` expired-tier into `purge::forget()`.
- `purge::forget` already calls `nexus.delete_node_by_event_id()` +
  `archive.drop_event()` — only the engine callback is missing.
- **Verify**: consolidation `age > 365` → after pruner run, event_id
  absent in Nexus (Cypher), Meili (doc count), Vectorizer (vector
  count), archive (grep fails).

### Phase 6 — Observability dashboard (gate: errors visible in real-time)
- Extend `GET /v1/retention/state` with `recent_errors[]`.
- Persist errors in new table `cron_errors(id, job_name, error_msg,
  occurred_at, is_resolved)`.
- GUI subscribes to `/v1/retention/state` and surfaces alerts.
- **Verify**: inject Vectorizer outage → dashboard shows red alert in
  < 5s; clears on recovery.

### Per-phase criteria

| Phase | Verifiable criterion |
|-------|---------------------|
| 1 | Nightly archive purge; no manual `rm -rf` |
| 2 | Tool-call digest deletes by default |
| 3 | No live-file ERRORs in peak hours |
| 4 | Vacuum never fails silently |
| 5 | Expired consolidations cascade across all stores |
| 6 | Dashboard shows failures in real-time |

**Parallelizable**: phases 1, 2, 3 independent (all in
`cortex-workers`). Phase 4 orthogonal. Phase 5 depends on 1. Phase 6 is
independent UI work.
