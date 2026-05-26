# Spec-11 lane projection contract: top-level fields, not nested ext
**Source**: manual
**Date**: 2026-05-03
**Related Task**: phase11k_governance_lane_projection
**Tags**: phase11k, spec-11, spec-08, lane-projection, meili, fulltext, architecture
The cortex-api `MeiliKeywordLane.project` flattens unknown TOP-LEVEL Meili document fields into `extras_raw` (via `#[serde(flatten)]`), then copies those keys into `LaneHit.extras` per `LANE_EXTRAS_KEYS`. The orchestrator's overlay derivers (`derive_decisions`, `derive_laws`, `derive_similar_turns`) read off `extras` directly.

Implication for write-side workers: contract keys (`decision_id`, `decision_title`, `decision_status`, `law_id`, `turn_id`, …) MUST be stamped at the top level of the Meili document. Nesting them under `ext.decision.*` / `ext.law_violation.*` (which the dashboard's filterable schema uses) is invisible to the lane projection — the read side never reaches into `ext.*`.

Phase11k §1 closed this by adding the keys as `Option<String>` fields on `cortex-workers::fulltext::Document` AND adding them to `filterableAttributes` in settings v5 so Meili indexes them. The dashboard's facet view continues to read `ext.decision.*` for back-compat; the lane projection contract reads the new top-level fields. Both paths coexist.

Reader-side never touches midway transforms via `filterableAttributes` — those control INDEXING, not projection. The only thing that surfaces a key into `LaneHit.extras` is its presence at the top of the indexed document.