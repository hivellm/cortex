# Cortex System Analysis - Findings Report

**Model:** z-ai/glm-5.1  
**Date:** 2026-05-06  
**Scope:** Full codebase audit across 10 crates

---

## Executive Summary

Cortex is a well-architected system with strong foundations, comprehensive documentation, and thoughtful design. The analysis identified **47 findings** across 6 severity levels. The majority are low-to-medium impact refactoring opportunities. Only **2 critical** and **9 high-severity** issues require immediate attention.

---

## F-001: Missing JSON Schemas for `knowledge` and `learning` Event Types

| Attribute | Value |
|-----------|-------|
| **Severity** | Critical |
| **Category** | Correctness Bug |
| **Location** | `crates/cortex-core/schemas/kinds/` |
| **Evidence** | `cortex-core/src/validate.rs:82-95` |
| **Impact** | Validation silently fails for `Kind::Knowledge` and `Kind::Learning` events |
| **Confidence** | High |

### Description

The `Kind` enum includes `Knowledge` and `Learning` variants, and `KnowledgePayload`/`LearningPayload` structs are defined in `events.rs`. However, `knowledge.schema.json` and `learning.schema.json` are missing from `schemas/kinds/`. The `Validator::new()` method compiles schemas for 10 kinds but omits these two.

### Evidence

```rust
// cortex-core/src/validate.rs:82-95
fn new() -> Self {
    let mut validators = HashMap::new();
    for kind in &["turn", "tool_call", "agent_call", "memory", 
                  "decision", "analysis", "law_violation", "artifact",
                  "consolidation", "topic_card"] {
        // Missing: "knowledge", "learning"
    }
}
```

### Recommendation

1. Create `schemas/kinds/knowledge.schema.json` and `schemas/kinds/learning.schema.json`
2. Update `Validator::new()` to compile these schemas
3. Add integration tests for `Kind::Knowledge` and `Kind::Learning` validation

---

## F-002: Missing Meilisearch Index Definitions for `consolidations` and `topic_cards`

| Attribute | Value |
|-----------|-------|
| **Severity** | Critical |
| **Category** | Correctness Bug |
| **Location** | `crates/cortex-storage/src/fulltext.rs:29-83` |
| **Evidence** | Compare `fulltext::INDEXES` with `names::ALL_INDEXES` |
| **Impact** | Meilisearch bootstrap cannot configure consolidations/topic_cards indexes |
| **Confidence** | High |

### Description

`names.rs` declares `INDEX_CONSOLIDATIONS` and `INDEX_TOPIC_CARDS` and includes them in `ALL_INDEXES`. However, `fulltext::INDEXES` array has no corresponding entries. No settings JSON files exist in `schemas/meili/` for these indexes.

### Recommendation

1. Add `IndexSchema` entries for consolidations and topic_cards to `fulltext::INDEXES`
2. Create `schemas/meili/consolidations.settings.json` and `schemas/meili/topic_cards.settings.json`
3. Add integration test verifying `INDEXES` covers `ALL_INDEXES`

---

## F-003: Massive Cross-Module Code Duplication (Synap Infrastructure)

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Code Duplication |
| **Location** | `crates/cortex-workers/src/{classifier,embedder,fulltext,graph}/worker.rs` |
| **Evidence** | ~1,500 duplicated lines across 4 files |
| **Impact** | Bug fixes must be replicated 4x; drift risk |
| **Confidence** | High |

### Description

The following types are fully duplicated across all 4 worker modules:
- `SynapConsumer` trait
- `SynapPublisher` trait
- `ConsumedMessage` struct
- `OffsetTracker` struct + impl
- `BackpressureState` struct + impl (3/4 workers)
- `SynapHandle` struct + impl
- `LiveSynapConsumer` struct + impl
- `LiveSynapPublisher` struct + impl
- `MemorySynapConsumer` struct + impl
- `MemorySynapPublisher` struct + impl

The graph worker already has persistent-offset support (`OffsetTracker::seed()`, `LiveSynapConsumer::with_persistent_offset()`) that the other three lack.

### Recommendation

Extract all Synap infrastructure into `src/synap/mod.rs` with a single set of types. Each worker re-exports what it needs.

---

## F-004: Envelope Uses Raw `String` for Typed ID Fields

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Type Safety |
| **Location** | `crates/cortex-core/src/events.rs:16,25` |
| **Evidence** | Compare with `cortex-core/src/ids.rs:15-74` |
| **Impact** | Invalid ULID strings can be silently stored |
| **Confidence** | High |

### Description

`Envelope` uses `String` for `event_id` and `session_id` despite strongly-typed `EventId`/`SessionId` wrappers existing in `ids.rs`. The typed wrappers are defined but bypassed by the core data structures.

### Recommendation

Replace `Envelope.event_id: String` with `event_id: EventId` and `session_id: String` with `session_id: SessionId`. Update serialization to handle the wrapper transparently.

---

## F-005: Envelope.payload is Untyped `serde_json::Value`

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Type Safety |
| **Location** | `crates/cortex-core/src/events.rs:38` |
| **Evidence** | Typed payloads exist but are disconnected |
| **Impact** | No compile-time guarantee that Kind carries correct payload shape |
| **Confidence** | High |

### Description

All typed payload structs (`Turn`, `ToolCall`, `AgentCall`, `MemoryPayload`, etc.) exist but are never used in the `Envelope`. The `payload` field is `serde_json::Value`, requiring runtime downcasting with no type-level enforcement.

### Recommendation

Consider one of:
1. Generic `Envelope<T>` with `payload: T`
2. Enum `Payload` with variants for each kind
3. Runtime validation that enforces kind-to-payload mapping

---

## F-006: Cross-Field Validators Not Called from `validate_event()`

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Correctness Bug |
| **Location** | `crates/cortex-core/src/validate.rs:111-146` |
| **Evidence** | `validate_consolidation_payload` and `validate_topic_card_payload` are standalone |
| **Impact** | Cross-field invariants silently unenforced |
| **Confidence** | High |

### Description

`validate_consolidation_payload()` and `validate_topic_card_payload()` are standalone functions. The main `validate_event()` runs JSON Schema validation but does not invoke these cross-field validators automatically.

### Recommendation

Wire cross-field validators into `validate_event()` when `kind == "consolidation"` or `kind == "topic_card"`.

---

## F-007: `vocab.rs` KIND_IDS Out of Sync with `Kind` Enum

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Correctness Bug |
| **Location** | `crates/cortex-core/src/vocab.rs:21-30` |
| **Evidence** | Enum has 12 variants; constant lists 8 |
| **Impact** | Downstream vocabulary lookups fail for 4 kinds |
| **Confidence** | High |

### Description

`KIND_IDS` lists only 8 kinds but `Kind` enum has 12 (`knowledge`, `learning`, `consolidation`, `topic_card` missing).

### Recommendation

Update `KIND_IDS` and add compile-time or test-time verification that it stays in sync.

---

## F-008: Race Condition in `record_cron_run`

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Concurrency Bug |
| **Location** | `crates/cortex-storage/src/metadata.rs:565-598` |
| **Evidence** | SELECT then UPDATE on `failure_streak` |
| **Impact** | TOCTOU race if concurrent callers update same job |
| **Confidence** | Medium |

### Description

`record_cron_run` performs a read-then-write on `failure_streak` (SELECT at L566-575, then UPDATE). This is not atomic and subject to TOCTOU races.

### Recommendation

Use a single SQL statement:
```sql
UPDATE cron_jobs SET failure_streak = CASE WHEN ? = 'failed' THEN failure_streak + 1 ELSE 0 END
```

---

## F-009: Inconsistent Poisoned Mutex Handling

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Reliability |
| **Location** | Multiple files across `cortex-workers` |
| **Evidence** | Various strategies: recovery, silent ignore, event drop |
| **Impact** | Data loss in embedder worker; inconsistent behavior across workers |
| **Confidence** | High |

### Description

| File | Strategy |
|------|----------|
| `classifier_worker/worker.rs:844-846` | Recovers via `into_inner()` |
| `embedder/worker.rs:561-563` | Silently returns `Ok(None)` — **event dropped** |
| `graph/worker.rs:854-858` | Returns `false` (treats as not-seen) |
| `cache.rs:39,53-55` | Silently ignores / returns `None` |

### Recommendation

Standardize on recovery via `into_inner()` with `tracing::warn!` logging. Never drop events silently.

---

## F-010: Unbounded Dedup Set Grows Without Limit

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Memory Leak |
| **Location** | All 4 worker modules |
| **Evidence** | `processed: Mutex<BTreeSet<String>>` with no eviction |
| **Impact** | Long-running workers accumulate unbounded memory |
| **Confidence** | High |

### Description

Each worker has a `processed: Mutex<BTreeSet<String>>` for at-least-once dedup. Entries are never evicted. A worker processing millions of events accumulates an ever-growing set.

### Recommendation

Use a bounded LRU cache or time-windowed set (evict entries older than N hours).

---

## F-011: Missing Supervisor Protection in 3/4 Workers

| Attribute | Value |
|-----------|-------|
| **Severity** | High |
| **Category** | Reliability |
| **Location** | `cortex-workers/src/{embedder,fulltext,graph}/worker.rs` |
| **Evidence** | Only `classifier_worker` has `consume_errors_consecutive` threshold |
| **Impact** | Persistent Synap errors cause infinite log-spam without container restart |
| **Confidence** | High |

### Description

The classifier worker has a supervisor that exits the process when `consume_errors_consecutive` exceeds a threshold. The embedder, fulltext, and graph workers have no such protection—they loop forever with 500ms back-off.

### Recommendation

Extract the supervisor logic into the shared Synap infrastructure and apply to all workers.

---

## F-012: SDK Bypass in vectorizer_lane and meili_lane

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Architecture |
| **Location** | `cortex-api/src/vectorizer_lane.rs`, `cortex-api/src/meili_lane.rs` |
| **Evidence** | Direct HTTP instead of SDK calls |
| **Impact** | Wire-shape drift between SDK and actual implementation |
| **Confidence** | High |

### Description

Both lanes bypass their respective SDKs for direct HTTP due to wire-shape mismatches. This creates a maintenance burden when upstream SDKs change.

### Recommendation

Contribute wire-shape fixes upstream or document the exact divergence reasons.

---

## F-013: `std::sync::RwLock` and `Mutex` in Async Context

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Performance |
| **Location** | Multiple files in `cortex-api` |
| **Evidence** | `acl.rs`, `orchestrator.rs`, `rate_limit.rs`, `query_rewrite.rs`, `audit.rs`, `loader_metrics.rs`, `analyzer.rs`, `audit_store.rs` |
| **Impact** | Can block tokio runtime |
| **Confidence** | High |

### Description

Several modules use `std::sync::RwLock` or `std::sync::Mutex` instead of `tokio::sync::*` equivalents. This can block the async runtime on contention.

### Recommendation

Audit each usage. For short-lived critical sections, `std::sync` is acceptable. For I/O-holding locks, migrate to `tokio::sync`.

---

## F-014: God Object `MetadataStore`

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Architecture |
| **Location** | `crates/cortex-storage/src/metadata.rs:33-1860` |
| **Evidence** | 25+ methods across 6 unrelated domains |
| **Impact** | Difficult to test and maintain |
| **Confidence** | High |

### Description

`MetadataStore` handles sessions, retention sweeps, classifier spend, cron jobs, consumer offsets, and bootstrap dedup. The file is 1860 lines.

### Recommendation

Decompose into focused sub-stores: `SessionStore`, `SweepStore`, `CronStore`, `SpendStore`, each accepting `&Connection`.

---

## F-015: DDL Duplication Between `schema.sql` and `apply_phase*_schema()`

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Maintenance |
| **Location** | `cortex-storage/src/metadata.rs:869-945` + `schemas/sqlite/schema.sql` |
| **Evidence** | Same CREATE TABLE statements in two places |
| **Impact** | Drift risk; schema inconsistencies |
| **Confidence** | High |

### Description

DDL for `bootstrap_seen`, `cron_jobs`, and rollup tables exists in both `schema.sql` and inline in `apply_phase*_schema()` functions.

### Recommendation

Use a single source of truth. Generate `apply_phase*_schema()` from `schema.sql` or use `include_str!`.

---

## F-016: Silent Error Swallowing in Archive Walker

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Error Handling |
| **Location** | `crates/cortex-storage/src/archive.rs:110-150` |
| **Evidence** | `read_dir`, `File::open`, `zstd::Decoder` errors silently ignored |
| **Impact** | Corrupt/unreadable files skipped without visibility |
| **Confidence** | High |

### Description

`walk_envelopes_dir` swallows I/O, decompression, and parsing errors with no logging or counters.

### Recommendation

Add `tracing::warn!` for each error type and increment a metric counter.

---

## F-017: `trim_payload` Ignores Budget Parameter

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Correctness |
| **Location** | `crates/cortex-workers/src/bin/graph-backfill.rs:495` |
| **Evidence** | `fn trim_payload(v: &Value, _budget: usize)` |
| **Impact** | Budget constraint not enforced |
| **Confidence** | High |

### Description

The `_budget` parameter is accepted but never used. Hardcoded constants (256 char string limit, 16 element array cap) apply regardless.

### Recommendation

Remove unused parameter or implement budget-aware trimming.

---

## F-018: `InMemoryCache` Silently Swallows Errors

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Correctness |
| **Location** | `crates/cortex-workers/src/classifier/cache.rs:48-64` |
| **Evidence** | Returns `Ok(None)` on poisoned mutex |
| **Impact** | Cache appears empty; unnecessary re-classification |
| **Confidence** | High |

### Description

On poisoned mutex, `get()` returns `Ok(None)` and `put()` silently fails. The cache appears empty, causing every event to be re-classified.

### Recommendation

Recover from poison or propagate error. Log warning on recovery.

---

## F-019: Production `expect()` Calls Can Panic

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Reliability |
| **Location** | Multiple files in `cortex-adapter-claude-code` |
| **Evidence** | `publisher.rs:62`, `sync_paths.rs:105`, `wal.rs:65,87`, `install.rs:228,231` |
| **Impact** | Daemon crashes on unexpected conditions |
| **Confidence** | High |

### Description

Several `expect()` calls in production code can panic:
- `reqwest::Client::builder().build().expect(...)` on TLS misconfiguration
- `self.handle.lock().expect("wal mutex poisoned")` on panic in critical section
- JSON structure assumptions in `install.rs`

### Recommendation

Replace with proper error handling. Return `Result` or use graceful degradation.

---

## F-020: `Ordering::Relaxed` Used Throughout

| Attribute | Value |
|-----------|-------|
| **Severity** | Medium |
| **Category** | Concurrency |
| **Location** | Across `cortex-workers` |
| **Evidence** | 139 uses of `Ordering::Relaxed` |
| **Impact** | Potential memory ordering bugs |
| **Confidence** | Medium |

### Description

Most atomics use `Ordering::Relaxed`. For monotonic counters this is fine, but `compare_exchange` failure ordering should be `Acquire` not `Relaxed`. Budget tracker may allow concurrent threads to both see `ratio() < 1.0` and exceed budget.

### Recommendation

Audit each `Ordering::Relaxed` use. Use `SeqCst` for budget state-check-and-record.

---

## F-021 to F-047: Additional Findings (Summarized)

| ID | Severity | Category | Location | Description |
|----|----------|----------|----------|-------------|
| F-021 | Medium | Dead Code | `cortex-storage/Cargo.toml:19,23` | `anyhow` and `once_cell` unused |
| F-022 | Medium | Duplication | `cortex-workers/bin/*.rs` | `home_dir()`, `resolve_metadata_db_path()`, `require_api_key()` duplicated |
| F-023 | Medium | Duplication | `graph-backfill.rs` vs `nexus_client.rs` | Cypher string escaping duplicated |
| F-024 | Medium | Architecture | `cortex-api/acl.rs`, `audit.rs` | Silent lock-failure swallowing |
| F-025 | Medium | Architecture | `cortex-pre-thinking/formatter.rs:281` | `decision_byte_cap` unused |
| F-026 | Medium | Architecture | `cortex-workers/embedder/embedder.rs` | `EnrichedEvent` owned by wrong module |
| F-027 | Medium | Architecture | `cortex-storage/archive.rs` | I/O-heavy code in "layout" crate |
| F-028 | Medium | Test Gap | `cortex-core/tests/fixtures_roundtrip.rs:108-123` | Only 8/12 kinds have fixtures |
| F-029 | Medium | Test Gap | `cortex-storage/metadata.rs` | Zero cron_jobs test coverage |
| F-030 | Medium | Test Gap | `cortex-workers/embedder,fulltext,graph/worker.rs` | Worker loops untested |
| F-031 | Low | Code Quality | `cortex-core/events.rs:987` | File is 987 lines; should split |
| F-032 | Low | Code Quality | `cortex-core/vocab.rs:4` | References non-existent test file |
| F-033 | Low | Code Quality | `cortex-storage/names.rs:31-43` | `ALL_STREAMS` manually synced |
| F-034 | Low | Code Quality | `cortex-workers/classifier/statics.rs:272` | Triple payload serialization |
| F-035 | Low | Code Quality | `cortex-workers/classifier/haiku_cli.rs:383` | O(n*m) topic normalization |
| F-036 | Low | Dependency | `cortex-workers/Cargo.toml:113-125` | Tree-sitter parsers not feature-gated |
| F-037 | Low | Dependency | `cortex-workers/Cargo.toml:90` | `once_cell` redundant with `std::sync::OnceLock` |
| F-038 | Low | Dependency | `cortex-workers/Cargo.toml:81` | `base64` version mismatch (0.22 vs 0.21 transitive) |
| F-039 | Low | Architecture | `cortex-storage/cas.rs:121` | CAS borrows compression level from archive |
| F-040 | Low | Architecture | `cortex-storage/graph.rs:27-47` | `RELATIONSHIPS` declared but never used |
| F-041 | Low | Architecture | `cortex-workers/classifier_worker/worker.rs` | `NormalisedEvent` tool field lost |
| F-042 | Low | Code Quality | `cortex-core/bin/cli.rs:60` | `std::process::exit(2)` bypasses destructors |
| F-043 | Low | Code Quality | `cortex-core/redact.rs:121-124` | Dead code / misleading `continue` |
| F-044 | Low | Code Quality | `cortex-core/canonical_json.rs:43,47` | Serialization allocates unnecessarily |
| F-045 | Low | Test Gap | `cortex-mcp-server/tools.rs` | Most tools lack wiremock tests |
| F-046 | Low | Test Gap | `cortex-adapter-claude-code` | Retry logic, IPC listener untested |
| F-047 | Low | Test Gap | `cortex-health` | `probe_one` failure modes untested |

---

## Summary by Severity

| Severity | Count | Description |
|----------|-------|-------------|
| **Critical** | 2 | Missing JSON schemas; missing Meili index definitions |
| **High** | 9 | Type safety gaps; race conditions; memory leaks; duplication |
| **Medium** | 19 | Architecture issues; dead code; error handling gaps |
| **Low** | 17 | Code quality; test gaps; dependency issues |

---

## Prioritized Remediation Roadmap

### Phase 1: Critical Fixes (1-2 days)
1. F-001: Create `knowledge.schema.json` and `learning.schema.json`
2. F-002: Add consolidations/topic_cards to `fulltext::INDEXES`

### Phase 2: High-Priority Fixes (1-2 weeks)
3. F-003: Extract Synap infrastructure into shared module
4. F-004, F-005: Type-safe envelope and payload
5. F-006: Wire cross-field validators
6. F-007: Fix `KIND_IDS` sync
7. F-008: Fix `record_cron_run` race
8. F-009: Standardize poisoned mutex handling
9. F-010: Add bounded dedup set
10. F-011: Add supervisor to all workers

### Phase 3: Medium-Priority Refactors (2-4 weeks)
11. F-012: SDK bypass documentation or upstream contribution
12. F-013: Async lock audit
13. F-014: Decompose `MetadataStore`
14. F-015: DDL single source of truth
15. F-016 to F-030: Error handling and test gaps

### Phase 4: Low-Priority Improvements (ongoing)
16. F-031 to F-047: Code quality and test coverage
