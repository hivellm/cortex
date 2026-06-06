# 36 — Temporal Observability & Audit Schema

> **Status:** 🟡 P6 partially shipped (audit events emitted; dashboard panels pending) · **Owner:** Core team · **Depends on:** 31, 34
> **Phase:** phase18_tlb-timeline-branching

## Goal

Consolidate the audit-event envelope contract for phase18's temporal, branch, and cross-project axes into a single observability schema. Dashboard panels (§7.2) and the weekly digest (§7.3) consume these events to provide operators visibility into retrieval quality, temporal filtering impact, branch resolution patterns, and cross-project reference propagation.

## Scope

**In:**

- Four audit event kinds (`temporal_classification`, `temporal_classification_summary`, `branch_resolution`, `cross_project_propagation`) emitted on the `cortex_audit` tracing target.
- Field definitions (type, meaning) for each event; forward-compatibility contract (unknown `kind`s are ignorable).
- Reference to section-count metric (spec 35 §6.2) for temporal section visibility.
- Derived signals that panels compute from the events (% non-now queries, candidate filter ratios, branch usage, cross-project hit ratio).

**Out:**

- Dashboard panels themselves (§7.2 — the chartable interpretation of the events).
- Weekly digest materialization (§7.3 — rolling aggregation and alert logic).
- GUI timeline + branch + history views (§7.4–7.8 — the user-facing rendering layer).

## ADR cross-reference

- ADR-018 — epoch-second integer representation in transit; RFC-3339 or YYYY-MM-DD on storage.
- ADR-021 — branch merge strategy folds branch facts into main retrievals via MERGED_INTO edge walk.
- ADR-023 — `CROSS_PROJECT_REF` disjoint from SUPERSEDES / OBSOLETES / EVOLVES_FROM.

Specs 31 (temporal classifier), 34 (cross-project propagation), 35 (bundle sections) are the contract sources.

## Audit events

All events are emitted on `target: "cortex_audit"` at `tracing::Level::INFO`. Consumers MUST treat unknown future `kind` discriminators as ignorable; new events will be added to this schema over time without breaking readers.

### Event: `temporal_classification`

**Kind discriminator:** `kind = "temporal_classification"` (string)

Emitted once per fused hit during the temporal classifier wedge (spec 31 §4, orchestrator.rs:780–789).

| Field | Type | Meaning |
|-------|------|---------|
| `kind` | string | `"temporal_classification"` — envelope kind discriminator |
| `query_id` | string | Stable UUID identifying this search request across all per-lane + fusion + classifier events |
| `doc_id` | string | Entity identifier (e.g., `"DEC-014"`, `"MEM-42"`, `"cortex:decision/example"`) |
| `state` | string | Classifier's state determination: one of `valid`, `temporal`, `superseded`, `expired`, `not_yet_valid`, `abandoned` (see spec 31 §1) |
| `action` | string | Resulting action: one of `pass`, `boost`, `demote`, `drop` |
| `as_of_unix` | i64 | Epoch seconds (UTC) used for state evaluation; wall-clock now if request omitted `as_of` parameter |

**Contract (SHALL):**
- The orchestrator emits one `temporal_classification` event per hit in the fused set before the cross-encoder reranker.
- `state` and `action` deterministically derive from (doc_id's bitemporal fields, as_of_unix, request's include_* flags, TemporalConfig).
- `doc_id` matches the source entity's primary key (used for deduplication, provenance tracking).

### Event: `temporal_classification_summary`

**Kind discriminator:** `kind = "temporal_classification_summary"` (string)

Emitted once per request after all per-hit `temporal_classification` events, rolling up state + action distributions (spec 31 §4, orchestrator.rs:811–826).

| Field | Type | Meaning |
|-------|------|---------|
| `kind` | string | `"temporal_classification_summary"` — envelope kind discriminator |
| `query_id` | string | Same UUID as the per-hit events |
| `evaluated` | u32 | Total count of candidates passed to the classifier |
| `valid` | u32 | Count of candidates classified as `valid` state |
| `temporal` | u32 | Count classified as `temporal` state (in the recency boost window) |
| `superseded` | u32 | Count classified as `superseded` state |
| `expired` | u32 | Count classified as `expired` state |
| `not_yet_valid` | u32 | Count classified as `not_yet_valid` state |
| `abandoned` | u32 | Count classified as `abandoned` state (lifecycle = "abandoned") |
| `drops` | u32 | Total count of `action = drop` decisions |
| `boosts` | u32 | Total count of `action = boost` decisions |
| `demotes` | u32 | Total count of `action = demote` decisions |

**Contract (SHALL):**
- The sum `drops + boosts + demotes ≤ evaluated` (the inequality accounts for `pass` actions, which are not tallied separately).
- The sum `valid + temporal + superseded + expired + not_yet_valid + abandoned == evaluated` (every candidate has exactly one state).
- One summary event per request (identified by `query_id`).

### Event: `branch_resolution`

**Kind discriminator:** `kind = "branch_resolution"` (string)

Emitted once per request to record the branch ancestry chain walked during request processing (spec 31 §5, orchestrator.rs:852–859).

| Field | Type | Meaning |
|-------|------|---------|
| `kind` | string | `"branch_resolution"` — envelope kind discriminator |
| `query_id` | string | Same UUID as per-hit and summary events |
| `branch` | string | Composite branch identifier in format `<project>:<branch>` (e.g., `"cortex:feat/timeline"`, `"cortex:main"`) — the requested or default branch |
| `ancestry_chain` | Vec<String> / JSON array | Ordered list of composite branch IDs from requested branch upward to root; today's default is singleton `[<project>:main]` (phase18 §3.5 ships the parent-map helper; phase18 §4 lands the per-request cache for full walks) |

**Contract (SHALL):**
- The first element of `ancestry_chain` is the `branch` field value (the requested/default endpoint).
- Each subsequent element is a parent or ancestor of the previous (walking toward the root main branch).
- The last element's branch name is always `"main"` (canonical root).
- If the request does not supply an explicit branch, the default is `<project>:main`.

### Event: `cross_project_propagation`

**Kind discriminator:** `kind = "cross_project_propagation"` (string)

Emitted once per request when cross-project propagation is enabled and the walk completes (spec 34 §2.2, orchestrator.rs:627–638). On error, the error variant is emitted instead (orchestrator.rs:539–546).

#### Success variant

| Field | Type | Meaning |
|-------|------|---------|
| `kind` | string | `"cross_project_propagation"` — envelope kind discriminator |
| `query_id` | string | Same UUID as temporal classification events |
| `active_project` | string | Project ID of the queried corpus (e.g., `"cortex"`, `"nexus"`) |
| `requested` | u32 | Count of sibling projects explicitly requested by the user via `request.projects` array |
| `discovered` | u32 | Count of CROSS_PROJECT_REF edges returned by the graph template walk (before filtering) |
| `kept` | u32 | Count of edges whose source_project matches a requested sibling (post-filter, pre-temporal-constraint) |
| `propagated` | u32 | Count of candidate hits fused into the result after temporal classifier re-application |
| `dropped` | u32 | Count of edges that passed the sibling filter but were dropped by temporal classifier or deduplication; equals `kept - propagated` |

**Contract (SHALL):**
- `discovered ≥ kept ≥ propagated` (monotonic filtering: walk → sibling filter → temporal constraint + dedup).
- `dropped = kept - propagated` (arithmetic invariant for audit trails).
- One success event per request when the graph walk succeeds.

#### Error variant

| Field | Type | Meaning |
|-------|------|---------|
| `kind` | string | `"cross_project_propagation"` — envelope kind discriminator |
| `query_id` | string | Same UUID as temporal classification events |
| `active_project` | string | Project ID of the queried corpus |
| `error` | string | Error message or type from the graph query layer (e.g., `"graph connection timeout"`) |

**Contract (SHALL):**
- Emitted when the graph template walk fails (spec 34 §2.2, orchestrator.rs:539–546).
- Propagation is aborted; no cross-project references are fused into the result.
- The error field enables operators to diagnose graph layer health without parsing logs.

## Derived signals

Dashboard panels (§7.2) compute the following metrics FROM the audit events without modifying or extending the event schema:

| Signal | Numerator | Denominator | Source Event | Use case |
|--------|-----------|-------------|--------------|----------|
| **% queries with non-`now` `as_of`** | count(requests where as_of_unix ≠ wall_clock) | count(all requests) | `temporal_classification` (compare `as_of_unix` against current time) | Measure adoption of time-travel search; surface trend toward temporal queries |
| **% candidates dropped by classifier** | sum(`drops` from all summaries) | sum(`evaluated` from all summaries) | `temporal_classification_summary` | Primary metric: effectiveness of temporal filtering |
| **Candidate state distribution** | per-state counts (e.g., `valid`, `superseded`, `expired`) | sum(`evaluated`) | `temporal_classification_summary` | Understand what portion of the index is currently valid vs. superseded/expired |
| **Action distribution** | per-action counts (`drops`, `boosts`, `demotes`, `pass`) | sum(`evaluated`) | `temporal_classification_summary` | Monitor reranking intensity and demote-vs-drop tradeoff |
| **Branch usage distribution** | frequency of each unique `branch` value | total request count | `branch_resolution` | Identify which branches are actively queried; flag stale/unused branches |
| **Cross-project hit ratio** | sum(`propagated`) across all CPP events | sum(`discovered`) across all CPP events | `cross_project_propagation` (success variant) | Measure what fraction of cross-project edges survive temporal + dedup filtering |
| **Cross-project error rate** | count(CPP error events) | count(all CPP events) | `cross_project_propagation` (error variant) | Monitor graph layer reliability when cross-project is enabled |

## Section-count metric reference

Spec 35 §6.2 defines a section-count emission function `observe_section_count(section: &str, count: u32)` that reports one sample per bundle section. Phase18 §7.1 makes three sections observable:

- `timeline_window` — candidates in the recency boost window
- `supersession_overlay` — candidates marked superseded but kept via `include_history`
- `branch_context` — candidates from a non-main branch

The metric backend (Prometheus / metrics exporter) receives these as samples tagged by section name, enabling time-series visualization of section prevalence across the search corpus.

## Pinned tests

**Integration test — per-hit + summary + branch_resolution audit trail:**

`crates/cortex-api/tests/temporal_audit_it.rs` — recording subscriber validates the complete envelope shape:
- Emits N `temporal_classification` events (one per fused hit)
- Emits 1 `temporal_classification_summary` event (rollup)
- Emits 1 `branch_resolution` event (ancestry chain)
- All events share the same `query_id`

**Unit tests — temporal classifier audit emission:**

`crates/cortex-api/src/search/orchestrator.rs::temporal_classification_tests` — validates:
- State determination (state machine priority per spec 31 §1)
- Action mapping (state + flags → action per spec 31 §2)
- Audit envelope field population (query_id, doc_id, state, action, as_of_unix)

**Integration test — cross-project propagation audit:**

`crates/cortex-api/tests/cross_project_it.rs` — validates the CPP event:
- Success variant: `discovered`, `kept`, `propagated` counts are monotonic
- Temporal constraint applied (valid_to before as_of drops edges)
- Source_project filtering (non-requested siblings omitted from `kept` count)
- Error variant emitted on graph query failure

**Unit tests — cross-project propagation logic:**

`crates/cortex-api/src/search/orchestrator.rs::cross_project_propagation_tests` (3 tests) — validates:
- Edge filtering (sibling whitelist + temporal constraint)
- Dropped count arithmetic (`kept - propagated`)
- Audit envelope shape (success and error variants)

## Consumers

- **Dashboard panels (§7.2):** stream `temporal_classification_summary` + `branch_resolution` + `cross_project_propagation` (success variant) for charting (dropped %, state distribution, branch usage, CPP hit ratio).
- **Weekly digest (§7.3):** aggregate per-hour or per-day buckets of summaries + error count; surface regressions (e.g., drop rate spike, CPP error rate > 5%).
- **Operator CLI tail (runtime observability):** `cortex logs --follow --filter 'target:cortex_audit'` surfaces the per-hit + summary stream for debugging "why was this candidate dropped?".
