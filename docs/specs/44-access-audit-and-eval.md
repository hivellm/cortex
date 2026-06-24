# Spec 44 — Access Audit & Eval

Phase21 §8–§9 — audit envelopes emitted by the ACL enforcement points,
the dashboard contract that surfaces aggregated metrics, and the eval/CI
gates that verify zero false-grants.

Related: [Spec 42 — Access Enforcement](42-access-enforcement.md),
[Spec 41 — Principal & RBAC](41-principal-and-rbac.md)

---

## 1. Audit envelope shapes

### 1.1 Per-hit event — `access_decision`

Emitted once per fused `LaneHit` by `apply_acl_wedge` in
`crates/cortex-api/src/search/orchestrator.rs`.

**Target**: `cortex_audit`  
**Level**: `INFO`  
**`kind` field**: `"access_decision"`

| Field | Type | Description |
|---|---|---|
| `kind` | `&str` | `"access_decision"` |
| `query_id` | `%str` | ULIDv4 correlation key for this request |
| `principal_id` | `%str` | Resolved identity; `"anonymous"` when unauthenticated |
| `doc_id` | `%str` | Lane hit document identifier |
| `fact_level` | `?Option<u8>` | Classification level of the fact (0–3); `None` = unclassified |
| `fact_compartments` | `%str` | Comma-separated compartment list; empty string when none |
| `clearance_level` | `u8` | Principal's Bell-LaPadula clearance level |
| `verdict` | `%str` | `"grant"` or `"deny"` |
| `reason` | `%str` | `"granted"`, `"above_clearance_level"`, or `"missing_compartment"` |

**SHALL requirement**: every call to `apply_acl_wedge` with a non-`None`
`AclContext` MUST emit one `access_decision` event per evaluated hit,
regardless of verdict.

**Pinned test**: `crates/cortex-api/tests/access_decision_audit_it.rs::access_decision_envelopes_fire_with_required_fields`

### 1.2 Per-request event — `access_decision_summary`

Emitted once per call to `apply_acl_wedge` (one per query request) after
all per-hit events.

**Target**: `cortex_audit`  
**Level**: `INFO`  
**`kind` field**: `"access_decision_summary"`

| Field | Type | Description |
|---|---|---|
| `kind` | `&str` | `"access_decision_summary"` |
| `query_id` | `%str` | Same ULIDv4 as the per-hit events |
| `principal_id` | `%str` | Same resolved identity |
| `evaluated` | `u64` | Total hits evaluated (granted + denied) |
| `granted` | `u64` | Hits that passed `can_read` |
| `denied` | `u64` | Hits that failed `can_read` |

**Pinned test**: `crates/cortex-api/tests/access_decision_audit_it.rs::access_decision_envelopes_fire_with_required_fields`

### 1.3 Reason vocabulary

The `reason` field distinguishes denial cause so operators can differentiate
level violations from compartment violations in log analysis:

| Value | Meaning |
|---|---|
| `"granted"` | The principal passes both the level gate and all compartment checks. |
| `"above_clearance_level"` | `fact_level > principal.clearance_level`. |
| `"missing_compartment"` | Level gate passes but at least one required compartment is absent from `principal.compartment_grants`. |

**Pinned test**: `crates/cortex-api/tests/access_decision_audit_it.rs::access_decision_reason_distinguishes_level_vs_compartment_denial`

### 1.4 Principal ID threading

The `principal_id` is threaded from `run_with_principal` through
`run_with_acl` (as `Option<&str>`) into `apply_acl_wedge`. When the caller
uses `run()` (no principal), both `acl_override` and `principal_id` are
`None` — no audit events fire and no lattice check is performed.

**Pinned test**: `crates/cortex-api/tests/access_decision_audit_it.rs::access_decision_carries_principal_id`

---

## 2. Dashboard contract — `/v1/dashboard/acl-stats`

**Route**: `GET /v1/dashboard/acl-stats`  
**Handler**: `crates/cortex-api/src/dashboard/acl_stats.rs::acl_stats_handler`  
**State**: `DashboardState.acl_metrics: Option<Arc<AclMetrics>>`

### 2.1 Response body

```json
{
  "total_evaluated": 120,
  "total_granted": 95,
  "total_denied": 25,
  "denial_rate_over_time": [
    { "ts": 1750000000, "denied": 3, "evaluated": 12 }
  ],
  "classification_distribution": [
    { "repo": "cortex", "level": 0, "count": 80 },
    { "repo": "cortex", "level": 2, "count": 10 }
  ],
  "top_denied_principals": [
    { "principal_id": "alice", "denial_count": 20 }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `total_evaluated` | `u64` | Cumulative hits evaluated since boot. `0` when AC disabled. |
| `total_granted` | `u64` | Cumulative grants since boot. |
| `total_denied` | `u64` | Cumulative denials since boot. |
| `denial_rate_over_time` | `DenialRateBucket[]` | 5-minute buckets over the last 60 minutes; empty when no data. |
| `classification_distribution` | `ClassificationDistributionRow[]` | Per-`(repo, level)` counts derived from the keyword lane. |
| `top_denied_principals` | `TopDeniedPrincipal[]` | Top-10 by cumulative denial count, descending. |

### 2.2 `DenialRateBucket`

| Field | Type | Description |
|---|---|---|
| `ts` | `i64` | Unix epoch seconds for the bucket start (5-minute alignment). |
| `denied` | `u64` | Denied decisions in this bucket. |
| `evaluated` | `u64` | Total evaluated in this bucket. |

### 2.3 `ClassificationDistributionRow`

| Field | Type | Description |
|---|---|---|
| `repo` | `String` | Repository name (from lane hit `repo` field, `"unknown"` when absent). |
| `level` | `u8` | Classification level: 0=public, 1=internal, 2=confidential, 3=restricted. |
| `count` | `u64` | Hit count at this `(repo, level)`. |

### 2.4 `TopDeniedPrincipal`

| Field | Type | Description |
|---|---|---|
| `principal_id` | `String` | Principal identifier. |
| `denial_count` | `u64` | Cumulative denial count since boot. |

### 2.5 Disabled-AC behaviour

When `access_control.enabled = false` (the default), `AclMetrics` is not
wired (`DashboardState.acl_metrics = None`). The handler returns:

```json
{
  "total_evaluated": 0,
  "total_granted": 0,
  "total_denied": 0,
  "denial_rate_over_time": [],
  "classification_distribution": [ /* derived from keyword lane */ ],
  "top_denied_principals": []
}
```

Classification distribution is always populated from the keyword lane
regardless of AC state (it reflects the ingestion posture, not runtime
decisions).

### 2.6 Frontend types

Mirror types live in `gui/src/lib/api.ts`:
- `DenialRateBucket`
- `ClassificationDistributionRow`
- `TopDeniedPrincipal`
- `AclStatsBody`
- `api.aclStats()` fetcher

View: `gui/src/views/AclStats.tsx` (`AclStatsView` component).  
Tests: `gui/src/views/AclStats.test.tsx` (happy / empty / error, 3 vitest cases).

---

## 3. AclMetrics store

**File**: `crates/cortex-api/src/security/acl_metrics.rs`

Aggregating registry for post-fusion ACL decisions. Mirrors the
`TemporalMetrics` pattern: atomics for counters, a bounded ring buffer
(`VecDeque`, cap=10 000) for time-series queries.

| Method | Description |
|---|---|
| `record(ts_secs, verdict, principal_id, fact_level, reason, doc_id)` | Record one decision into the ring buffer and update counters. |
| `top_denied_principals(limit)` | Top-N denied principals sorted descending by denial count. |
| `denial_rate_over_time(window_minutes)` | 5-minute buckets over the last `window_minutes` minutes. |

The orchestrator holds `Option<Arc<AclMetrics>>`; `None` is a zero-cost
no-op (no allocation, no lock contention). The handle is instantiated in
`main.rs` when `access_control.enabled = true` and threaded via
`Orchestrator::with_acl_metrics()`.

**Pinned unit tests** (`acl_metrics.rs` internal `mod tests`):
- `record_bumps_counters` — atomics increment correctly for grant + deny.
- `top_denied_principals_sorts_descending` — descending order, correct counts.
- `grants_do_not_appear_in_denied_by_principal` — grants must not pollute the denial map.
- `rolling_window_evicts_oldest_when_full` — capacity=2 ring buffer evicts oldest.
- `denial_rate_over_time_excludes_records_outside_window` — only recent decisions appear.

---

## 4. Eval gates (Phase21 §9)

### 4.1 Golden access-control suite

**File**: `crates/cortex-eval/src/suite/access_control.rs`  
**Golden**: `tests/golden/access_control.csv`

The suite runs a matrix of `(principal_clearance, fact_level, compartments)` cases
through `can_read` and against the live post-fusion wedge. Each row is labelled
`visible` or `hidden`.

**Zero-false-grants acceptance gate**:

```
The system SHALL produce zero false-grants.
A false-grant is any case where a row labelled `hidden` appears in the
retrieval result for its associated principal.
```

The gate fails CI if `false_grant_count > 0`.  A false-negative (a
labelled-`visible` row absent from the result) is a recall loss but not a
security violation; it does not fail CI.

#### Scenario: zero false-grants across the clearance×level matrix

```
Given a corpus seeded with facts at every (level, compartment) combination
  And a principal at clearance 1 with grants = ["financial"]
When the golden suite runs all 16 matrix cases
Then every case labelled `hidden` must be absent from the result
  And false_grant_count must equal 0
```

**Pinned test file**: `crates/cortex-eval/src/suite/access_control.rs`

### 4.2 Adversarial leak probe

**Description**: A low-clearance principal (clearance=0, no compartments) exercises
every retrieval surface against a corpus seeded exclusively with `restricted`
(level=3) and compartment-gated facts. The probe MUST retrieve zero classified hits
from any surface.

**Surfaces probed**:
- `POST /v1/query` (orchestrator path)
- `POST /v1/search/keyword` (raw keyword proxy)
- `POST /v1/search/vector` (raw vector proxy)
- `POST /v1/search/graph` (neighbors mode)
- Pre-thinking bundle filter

**CI gate**: a single leak (any classified hit visible to the low-clearance
principal) fails CI. The check is structural — the test itself is the gate.

**SHALL requirement**:

```
The system SHALL guarantee that a principal with clearance_level=0 and
no compartment_grants retrieves zero hits from any surface when the entire
corpus is classified at level >= 1 or behind any compartment.
```

#### Scenario: adversarial probe finds no classified leaks

```
Given access_control.enabled = true
  And a corpus seeded entirely with restricted (level=3) and compartment-gated facts
  And a principal with clearance_level=0, compartment_grants=[]
When the adversarial probe exercises all five surfaces
Then every surface returns an empty result
  And the CI gate reports 0 leaks
```

**Pinned test**: `crates/cortex-eval/src/suite/access_control.rs` (adversarial section)

---

## 5. Pinned tests

| File | Count | What |
|---|---|---|
| `crates/cortex-api/tests/access_decision_audit_it.rs` | 3 | Audit envelope shapes (fields, reason codes, principal_id) |
| `crates/cortex-api/src/security/acl_metrics.rs` (unit) | 5 | AclMetrics store correctness |
| `gui/src/views/AclStats.test.tsx` | 3 | Dashboard view happy / empty / error |
| `crates/cortex-eval/src/suite/access_control.rs` (unit) | 10 | Zero-false-grants gate: predicate, CSV loader, report builder, acceptance |
| `crates/cortex-eval/tests/golden/access_control.csv` | 40 rows | Matrix: clearance×level×compartments; evaluated via `cortex-eval --suite access_control` |
| `crates/cortex-api/tests/adversarial_leak_probe_it.rs` | 3 | **Hard CI gate** — zero-clearance principal retrieves 0 hits from restricted corpus |

All tests MUST remain green. `cargo clippy --workspace -- -D warnings` MUST
pass with zero warnings.
