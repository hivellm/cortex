# 4. Cortex graph nodes carry their identity in Nexus's reserved _id slot

**Status**: proposed
**Date**: 2026-05-02
**Related Tasks**: phase11k_graph_correlations, phase11l_nexus-external-ids-migration

## Context

Phases phase4-phase11k shipped a synthetic `natural_key` convention for graph node identity (`repo|path|content_hash` for Artifact, `repo|language|qualified_name` for Symbol, `id` for Session/Turn/ToolCall/Decision/Memory/Analysis/Law/LawViolation, `name` for Repo, `path` for Spec). Each label declared a `IS UNIQUE` constraint on the chosen property, and every Cypher template upserted via `MERGE (n:Label { natural_key: $key }) SET n += $props`. This emulated external identity by hand because Nexus 2.0 had no reserved `_id` slot. Nexus 2.1 (phase9_external-node-ids + phase10 SDK live validation, 2026-05-02) added first-class external-id support: a reserved `_id` property accepts hash / uuid / str / bytes keys, an LMDB-backed `ExternalIdIndex` enforces uniqueness structurally, and the Cypher executor recognises `MATCH (n {_id: $x})` as an O(1) hash / O(log n) string index seek with `CREATE { _id: $x } ON CONFLICT MATCH|REPLACE|ERROR` modifiers.

## Decision

Cortex's graph layer adopts Nexus's reserved `_id` slot as the canonical node identity. Every NodeOp now carries `external_id: Option<String>` populated from the same canonical key string the legacy `natural_key` field stamps; every node Cypher template rewrites `MERGE { natural_key }` to `CREATE { _id } ON CONFLICT MATCH`; the five `*_natural_key` schema constraints are retired (the external-id index supersedes them); the `UNKNOWN_CONTENT_HASH = "*"` sentinel rewrites to a deterministic per-`(repo, path)` `pending|repo|path` form so the new uniqueness rule does not collapse unknown-hash siblings. The migration is structural — bootstrap-replay across all 17 indexed repos rebuilds the graph under the new keying.

## Alternatives Considered

- Status quo (synthetic natural_key MERGE): rejected — every upsert pays property-lookup cost forever, UNKNOWN_CONTENT_HASH ambiguity remains latent, cross-system joins still round-trip through property store, duplicate-code burden alongside Nexus 2.1.
- Per-label hash-prefix _id (sha256:-prefix Artifact, str:-prefix Symbol): considered but premature. Current passthrough still benefits from index seek; per-label format change can land later through the external_id_for_node helper that already factors out the rule.
- Online migration (rewrite natural_key to _id on existing nodes): rejected — Nexus 2.1 has no API to set _id post-creation; the only path is DELETE + CREATE per node which loses connected edges. Drop + bootstrap-replay is structurally cleaner.

## Consequences

Wins: query latency drops 5-15% on Artifact lookups (index seek replaces label-scan + property comparison); idempotent re-ingest at storage level (re-running cortex-bootstrap --graph-static produces zero new nodes on unchanged content); cross-system joins by hash become first-class via GET /nodes/by-external-id; seven schema-bootstrap constraint lines disappear; the latent UNKNOWN_CONTENT_HASH sentinel ambiguity is structurally fixed. Costs: full Cortex graph DB drop + bootstrap-replay required (15-25 min wall clock on 17-repo workspace); Nexus 2.1.0 minimum container version; downstream consumers that read `natural_key` off the wire must migrate to `_id`. Reassessment trigger: track Nexus query latency p95 on Artifact MERGE vs CREATE ON CONFLICT MATCH for one quarter post-migration; if the gap drops below 5% the legacy soft-fallback can be deleted; if the doctor's `audit_graph_patch_shape` check reports >1% legacy-shape envelopes 30 days post-cutover, reopen the §6.3 cleanup task to flush laggard producers.
