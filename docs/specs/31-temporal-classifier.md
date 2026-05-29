# 31 — Temporal Classifier

> **Status:** 🟡 P2 partially shipped (state machine + config) · **Owner:** Core team · **Depends on:** 11, 30
> **Phase:** phase18_tlb-timeline-branching

## Goal

Drop or demote candidates that the bitemporal axis (spec 30) marks
as historically irrelevant, so default retrievals stop surfacing
superseded ADRs / expired learnings / abandoned exploration paths.
The classifier is the load-bearing primitive that turns bitemporal
storage into a useful relevance signal.

## Scope

**In:**

- 6-variant state machine per design.md §2.2:
  `VALID | TEMPORAL | SUPERSEDED | EXPIRED | NOT_YET_VALID | ABANDONED`.
- Per-state action: `Pass {multiplier}`, `Drop`, `Demote {factor}`.
- `IncludeFlags` operator opt-ins that flip default `Drop` into
  `Demote`.
- Tunable `TemporalConfig` defaults (`enabled = true`,
  `temporal_window_days = 30`, `temporal_boost = 1.10`,
  `demote_factor = 0.5`).
- Orchestrator wiring after fusion, before the cross-encoder
  reranker (phase17 P2).
- Audit envelope per classification call (§3.9).

**Out:**

- Bitemporal schema + writers — spec 30.
- Branch surfaces (CLI / HTTP / MCP) — spec 32.
- Cross-project axis activation — spec 34.

## ADR cross-reference

- ADR-018 (time precision) — classifier inputs use epoch-second
  ints; second-precision storage round-trips without parse.
- ADR-021 (branch merge) — classifier folds branch facts into
  `main` retrievals via the `MERGED_INTO` edge walk per merge
  strategy.
- ADR-023 (edge semantics) — `SUPERSEDES` is what stamps
  `superseded_at`; the classifier reads that column to derive the
  `SUPERSEDED` state.

## 1. State machine

Priority order — first match wins, the comparison runs in sequence,
and the state never re-enters:

```text
1. SUPERSEDED       superseded_at_unix IS NOT NULL AND ≤ as_of
2. EXPIRED          valid_to_unix IS NOT NULL AND ≤ as_of
3. NOT_YET_VALID    valid_from_unix > as_of
4. ABANDONED        lifecycle == "abandoned"
5. TEMPORAL         valid_to_unix IS NOT NULL AND ≤ as_of + window
6. VALID            default
```

## 2. Action table

| state         | default action      | with `include_history` | with `include_future` | with `include_branches` |
|---------------|---------------------|------------------------|-----------------------|-------------------------|
| `VALID`       | `Pass {1.0}`        | unchanged              | unchanged             | unchanged               |
| `TEMPORAL`    | `Pass {boost}`      | unchanged              | unchanged             | unchanged               |
| `SUPERSEDED`  | `Drop`              | `Demote {factor}`      | unchanged             | unchanged               |
| `EXPIRED`     | `Drop`              | `Demote {factor}`      | unchanged             | unchanged               |
| `NOT_YET_VALID` | `Drop`            | unchanged              | `Demote {factor}`     | unchanged               |
| `ABANDONED`   | `Drop`              | unchanged              | unchanged             | `Demote {factor}`       |

`boost` = `TemporalConfig::temporal_boost` (1.10 default).
`factor` = `TemporalConfig::demote_factor` (0.5 default).

## 3. Config knobs (`cortex-config::TemporalConfig`)

```toml
[temporal]
enabled                   = true
include_history_default   = false
temporal_window_days      = 30
temporal_boost            = 1.10
demote_factor             = 0.5
```

Env mirrors live at `CORTEX_TEMPORAL_*`. The orchestrator reads
`enabled` once per request; flipping it requires a daemon restart
(no SIGHUP path for the classifier today).

## 4. Orchestrator wiring (§3.3, pending)

The orchestrator at
`crates/cortex-api/src/search/orchestrator.rs::run_lanes` calls the
classifier after `rrf_fuse` and before the cross-encoder reranker:

```text
fused = rrf_fuse(per_lane_hits, &cfg.fusion);
classified = fused.into_iter()
    .filter_map(|hit| {
        let candidate = build_candidate(&hit);
        let (state, action) = classifier::classify(
            &candidate, as_of_unix, flags, &cfg.temporal_into_classifier()
        );
        audit_emit(query_id, &hit, state, action);
        match action {
            Action::Drop => None,
            Action::Pass { multiplier } => Some(hit.with_multiplier(multiplier)),
            Action::Demote { factor } => Some(hit.with_multiplier(factor)),
        }
    });
reranked = cross_encoder_rerank(classified);
```

## 5. Audit envelope (§3.9)

Every classification call emits one envelope on the
`cortex-audit` channel:

```text
{
    "kind": "temporal_classification",
    "query_id": "01HZQRY00000000000000000A",
    "entity_id": "DEC-014",
    "state": "superseded",
    "action": "drop",
    "reason": "superseded_at <= as_of",
    "as_of": "2026-04-01T00:00:00Z",
    "branch": "cortex:main",
}
```

A second envelope (`branch_resolution`) carries the
ancestry-chain walk:

```text
{
    "kind": "branch_resolution",
    "query_id": "01HZQRY00000000000000000A",
    "branch": "cortex:feat/x",
    "ancestry_chain": ["cortex:feat/x", "cortex:main"],
}
```

## 6. Eval gates (§3.8, pending)

- CDC harness time-sensitive subset MRR@10 ≥ +10% over the
  classifier-disabled baseline.
- No regression on the time-insensitive subset (median drop ≤ 2%).
- The eval harness flips `TemporalConfig::enabled` to compare
  baselines without a rebuild.

## 7. Pinned tests

`crates/cortex-workers/src/temporal/classifier.rs::tests` (11):

- `default_lane_yields_valid_with_unit_multiplier`
- `superseded_drops_by_default_and_demotes_with_include_history`
- `expired_takes_precedence_over_temporal_window`
- `not_yet_valid_fires_when_valid_from_is_in_the_future`
- `abandoned_lifecycle_drops_by_default_and_demotes_with_include_branches`
- `temporal_state_applies_recency_boost`
- `temporal_state_skipped_when_valid_to_is_past_window`
- `state_priority_superseded_wins_over_expired_and_abandoned`
- `missing_columns_default_to_valid`
- `temporal_config_default_matches_design_doc`
- `temporal_state_as_str_round_trips`

`crates/cortex-config/src/sub.rs::tests_temporal_config` (3):

- `defaults_match_design_doc_2_2`
- `toml_round_trips_defaults`
- `partial_toml_keeps_serde_defaults_for_missing_fields`

`crates/cortex-workers/src/temporal/branch_filter.rs::tests` (7):

- `root_main_branch_chain_has_one_entry`
- `linear_chain_resolves_in_retrieval_order_leaf_first_root_last`
- `cycle_is_broken_at_the_repeated_node`
- `missing_parent_treats_branch_as_root`
- `compose_id_uses_colon_separator`
- `meili_branch_clause_renders_in_disjunction_shape`
- `meili_branch_clause_escapes_embedded_quotes`
