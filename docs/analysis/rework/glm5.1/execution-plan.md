# Cortex Rework Execution Plan

**Model:** z-ai/glm-5.1  
**Date:** 2026-05-06  
**Based on:** findings.md (47 findings)

---

## Guiding Principles

1. **Zero regressions**: Every phase includes a quality gate (type-check + lint + tests)
2. **Minimal blast radius**: Each task touches the fewest files possible
3. **Sequential within phase**: Tasks in the same phase are ordered by dependency
4. **Crate-first scoping**: Refactors are scoped to a single crate where possible

---

## Phase 1: Critical Correctness Fixes

**Goal**: Eliminate silent validation failures and missing infrastructure  
**Duration**: 1-2 days  
**Risk**: Low (additive changes only)

### Task 1.1: Create Missing JSON Schemas
**Finding**: F-001  
**Files to create**:
- `crates/cortex-core/schemas/kinds/knowledge.schema.json`
- `crates/cortex-core/schemas/kinds/learning.schema.json`

**Files to modify**:
- `crates/cortex-core/src/validate.rs` — add `"knowledge"` and `"learning"` to the validator map

**Verification**: `cargo test -p cortex-core -- validate_event` passes for all 12 kinds

### Task 1.2: Add Missing Meilisearch Index Definitions
**Finding**: F-002  
**Files to create**:
- `crates/cortex-storage/schemas/meili/consolidations.settings.json`
- `crates/cortex-storage/schemas/meili/topic_cards.settings.json`

**Files to modify**:
- `crates/cortex-storage/src/fulltext.rs` — add `IndexSchema` entries for consolidations and topic_cards

**Verification**: `cargo test -p cortex-storage` passes; integration test confirms `INDEXES` covers `ALL_INDEXES`

### Task 1.3: Fix `vocab.rs` KIND_IDS
**Finding**: F-007  
**Files to modify**:
- `crates/cortex-core/src/vocab.rs` — add `knowledge`, `learning`, `consolidation`, `topic_card`

**Verification**: Test that `KIND_IDS` length matches `Kind::variant_count()` (or equivalent)

---

## Phase 2: Type Safety & Validation

**Goal**: Make the event pipeline type-safe end-to-end  
**Duration**: 1-2 weeks  
**Risk**: Medium (breaking API changes within internal crates)

### Task 2.1: Type-Safe Envelope IDs
**Finding**: F-004  
**Files to modify**:
- `crates/cortex-core/src/events.rs` — change `event_id: String` to `event_id: EventId`, `session_id: String` to `session_id: SessionId`
- `crates/cortex-core/src/ids.rs` — add `Serialize`/`Deserialize` impls if missing
- All downstream crates that construct `Envelope` directly

**Verification**: `cargo build --workspace` + `cargo test --workspace`

### Task 2.2: Wire Cross-Field Validators
**Finding**: F-006  
**Files to modify**:
- `crates/cortex-core/src/validate.rs` — call `validate_consolidation_payload()` and `validate_topic_card_payload()` from within `validate_event()`

**Verification**: Test that calling `validate_event()` on a consolidation envelope with invalid cross-field constraints returns errors

### Task 2.3: Typed Payload (Design Phase)
**Finding**: F-005  
**Output**: Design document with chosen approach (generic, enum, or runtime)

**This is a design-only task**. Implementation goes to Phase 5.

---

## Phase 3: Worker Infrastructure Consolidation

**Goal**: Eliminate ~1,500 lines of duplication across workers  
**Duration**: 1-2 weeks  
**Risk**: Medium (refactoring core worker infrastructure)

### Task 3.1: Extract Synap Infrastructure Module
**Finding**: F-003  
**Steps**:
1. Create `crates/cortex-workers/src/synap/mod.rs`
2. Move `SynapConsumer`, `SynapPublisher`, `ConsumedMessage`, `OffsetTracker`, `BackpressureState`, `SynapHandle`, `LiveSynapConsumer`, `LiveSynapPublisher`, `MemorySynapConsumer`, `MemorySynapPublisher` into the new module
3. Include persistent-offset support from graph worker (making it available to all)
4. Update all 4 worker modules to `use crate::synap::*`
5. Delete duplicated code from each worker

**Verification**: `cargo test -p cortex-workers` — all existing tests pass unchanged

### Task 3.2: Extract Bin-File Helpers
**Finding**: F-022  
**Steps**:
1. Create `crates/cortex-workers/src/bin_support/mod.rs`
2. Move `home_dir()`, `resolve_metadata_db_path()`, `require_api_key()`, `build_summarisers()` into it
3. Update all bin files to import from `bin_support`

**Verification**: `cargo test -p cortex-workers`

### Task 3.3: Add Supervisor to All Workers
**Finding**: F-011  
**Steps**:
1. Extract supervisor logic from classifier worker into shared module
2. Apply to embedder, fulltext, and graph workers

**Verification**: Integration test that a worker exits after N consecutive failures

### Task 3.4: Standardize Poisoned Mutex Handling
**Finding**: F-009  
**Steps**:
1. Add `tracing::warn!` on every poison recovery
2. Change `embedder/worker.rs:561-563` from `Ok(None)` to `into_inner()` recovery
3. Change `cache.rs` to recover instead of returning `None`

**Verification**: `cargo test -p cortex-workers`

### Task 3.5: Bounded Dedup Set
**Finding**: F-010  
**Steps**:
1. Replace `BTreeSet<String>` with a time-windowed eviction set (e.g., `DashMap` with timestamps, or a bounded LRU)
2. Set default retention window to 24 hours
3. Apply to all 4 workers

**Verification**: Unit test that set evicts old entries

---

## Phase 4: Reliability & Concurrency

**Goal**: Fix race conditions and improve error handling  
**Duration**: 1 week  
**Risk**: Low-Medium

### Task 4.1: Fix `record_cron_run` Race
**Finding**: F-008  
**Files to modify**:
- `crates/cortex-storage/src/metadata.rs` — rewrite `record_cron_run` to use single atomic SQL statement

**Verification**: Test concurrent calls produce correct `failure_streak` values

### Task 4.2: Audit `Ordering::Relaxed` Usage
**Finding**: F-020  
**Steps**:
1. Catalog all 139 uses
2. Change `compare_exchange` failure ordering to `Acquire`
3. Change budget tracker to `SeqCst` or `Mutex`

**Verification**: `cargo test --workspace`

### Task 4.3: Fix Production `expect()` Calls
**Finding**: F-019  
**Files to modify**:
- `crates/cortex-adapter-claude-code/src/publisher.rs`
- `crates/cortex-adapter-claude-code/src/sync_paths.rs`
- `crates/cortex-adapter-claude-code/src/wal.rs`
- `crates/cortex-adapter-claude-code/src/install.rs`

**Verification**: `cargo test -p cortex-adapter-claude-code`

### Task 4.4: Archive Walker Error Visibility
**Finding**: F-016  
**Files to modify**:
- `crates/cortex-storage/src/archive.rs` — add `tracing::warn!` and metric counters

**Verification**: Test with corrupt file produces warning log

---

## Phase 5: Architecture Improvements

**Goal**: Reduce technical debt in larger modules  
**Duration**: 2-4 weeks  
**Risk**: Medium

### Task 5.1: Decompose `MetadataStore`
**Finding**: F-014  
**Steps**:
1. Extract `CronStore`, `SpendStore`, `SessionStore`, `SweepStore`
2. Each sub-store accepts `&Connection`
3. `MetadataStore` composes them
4. Add cron_jobs tests (F-029)

**Verification**: `cargo test -p cortex-storage`

### Task 5.2: DDL Single Source of Truth
**Finding**: F-015  
**Steps**:
1. Keep `schema.sql` as single source
2. Use `include_str!` in `apply_phase*_schema()`
3. Or generate from `schema.sql` at build time

**Verification**: Diff check that inline DDL matches `schema.sql`

### Task 5.3: Move `EnrichedEvent` to Shared Location
**Finding**: F-026  
**Steps**:
1. Move `EnrichedEvent` from `cortex-workers/embedder` to `cortex-core`
2. Update all references

**Verification**: `cargo build --workspace`

### Task 5.4: Typed Payload Implementation
**Finding**: F-005 (design from Task 2.3)  
**Steps**: Implement chosen design

**Verification**: `cargo test --workspace`

---

## Phase 6: Test Coverage

**Goal**: Close critical test gaps  
**Duration**: 2-3 weeks  
**Risk**: Low

### Task 6.1: Core Fixture Coverage
**Finding**: F-028  
**Steps**: Add fixture files for `knowledge`, `learning`, `consolidation`, `topic_card`

### Task 6.2: Worker Loop Tests
**Finding**: F-030  
**Steps**: Add unit tests for `run_once`/`handle_message`/`handle_batch` in embedder, fulltext, graph workers

### Task 6.3: Cron Jobs Tests
**Finding**: F-029  
**Steps**: Add tests for `upsert_cron_job_if_absent`, `record_cron_run`, `select_due_cron_jobs`, `list_cron_jobs`

### Task 6.4: MCP Tool Wiremock Tests
**Finding**: F-045  
**Steps**: Add wiremock-based tests for `QueryTool`, `PreThinkingTool`, search tools

### Task 6.5: CAS Extended Tests
**Finding**: From storage audit  
**Steps**: Add tests for `select_vacuumable`, `delete_blobs`, `HashMismatch`, `total_blob_count`

---

## Phase 7: Code Quality & Cleanup

**Goal**: Address low-severity findings  
**Duration**: Ongoing  
**Risk**: Low

### Task 7.1: Remove Dead Dependencies
**Findings**: F-021, F-037, F-038  
- Remove `anyhow` and `once_cell` from `cortex-storage/Cargo.toml`
- Replace `once_cell` with `std::sync::OnceLock` in `cortex-core`
- Gate tree-sitter parsers behind feature flag in `cortex-workers`

### Task 7.2: Split `events.rs`
**Finding**: F-031  
- Move per-kind payloads into `payloads/` submodule

### Task 7.3: Fix Cypher Duplication
**Finding**: F-023  
- Make `cypher_string_literal` and `render_node_merge` `pub(crate)` in `nexus_client.rs`
- Import from backfill binary

### Task 7.4: Implement `decision_byte_cap`
**Finding**: F-025  
- Implement the truncation step in `formatter.rs`

---

## Dependency Graph

```
Phase 1 (Critical) → Phase 2 (Type Safety) → Phase 5 (Architecture)
                    → Phase 3 (Workers)     → Phase 5
                    → Phase 4 (Reliability)
Phase 6 (Tests) can run in parallel with Phases 3-5
Phase 7 (Cleanup) runs after all others
```

---

## Estimated Effort

| Phase | Duration | Complexity | Files Touched |
|-------|----------|------------|---------------|
| Phase 1 | 1-2 days | Low | ~8 |
| Phase 2 | 1-2 weeks | Medium | ~15 |
| Phase 3 | 1-2 weeks | Medium | ~25 |
| Phase 4 | 1 week | Low-Medium | ~10 |
| Phase 5 | 2-4 weeks | Medium-High | ~30 |
| Phase 6 | 2-3 weeks | Low | ~20 |
| Phase 7 | Ongoing | Low | ~15 |

**Total estimated effort**: 8-14 weeks for full rework
